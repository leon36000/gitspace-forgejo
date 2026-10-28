#!/usr/bin/env python3
"""Run Forgejo-emitted issue and pull-request webhooks through the receiver."""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import http.client
import json
import os
from pathlib import Path
import re
import secrets
import signal
import shutil
import socket
import sqlite3
import stat
import subprocess
import sys
import time
from typing import Any
from urllib.parse import quote

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_webhook_reconcile_e2e as helpers  # noqa: E402

OWNER = helpers.OWNER
REPOSITORY = helpers.REPOSITORY
READER = helpers.READER
CONTRIBUTOR = "forge-review-author"
ORIGIN = helpers.ORIGIN
FORGEJO_VERSION = "17.0.0+gitea-1.22.0"
RECONCILE_SINGLE_SCHEMA_VERSION = 19
RECONCILE_BATCH_SCHEMA_VERSION = 23
DELIVERY_ID_RE = re.compile(r"^[A-Za-z0-9_-]{1,128}$")
U64_MAX = (1 << 64) - 1
MAX_JOURNAL_BYTES = 4 * 1024 * 1024
DEFAULT_WEBHOOK_JOURNAL_WAIT_SECONDS = 20.0
COMMIT_SHA_RE = re.compile(r"^(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})$")
FORGEJO_V17_PULL_REQUEST_ACTIONS = frozenset(
    {
        "opened",
        "closed",
        "reopened",
        "edited",
        "assigned",
        "unassigned",
        "label_updated",
        "label_cleared",
        "milestoned",
        "demilestoned",
        "synchronized",
        "review_requested",
        "review_request_removed",
    }
)
HOOK_EVENT_SELECTORS = ("issues", "pull_request")
ISSUE_HOOK_EVENTS = frozenset(
    {
        "issues",
        "issue_assign",
        "issue_label",
        "issue_milestone",
        "issue_comment",
    }
)
PULL_REQUEST_HOOK_EVENTS = frozenset(
    {
        "pull_request",
        "pull_request_assign",
        "pull_request_label",
        "pull_request_milestone",
        "pull_request_comment",
        "pull_request_review_approved",
        "pull_request_review_rejected",
        "pull_request_review_comment",
        "pull_request_sync",
        "pull_request_review_request",
    }
)
RELEASE_HOOK_EVENTS = frozenset({"release"})
HOOK_EVENT_GROUPS = {
    "issues": ISSUE_HOOK_EVENTS,
    "pull_request": PULL_REQUEST_HOOK_EVENTS,
    "release": RELEASE_HOOK_EVENTS,
    "create": frozenset({"create"}),
    "push": frozenset({"push"}),
    "delete": frozenset({"delete"}),
}


def effective_hook_events(event_selectors: tuple[str, ...]) -> frozenset[str]:
    if (
        not event_selectors
        or len(event_selectors) != len(set(event_selectors))
        or any(selector not in HOOK_EVENT_GROUPS for selector in event_selectors)
    ):
        raise ValueError("unsupported or duplicate Forgejo webhook event selector")
    return frozenset().union(*(HOOK_EVENT_GROUPS[selector] for selector in event_selectors))


def ordinary_issue_comment_delivery_count(
    enabled: bool, *, edited: bool = False, deleted: bool = False
) -> int:
    if (edited or deleted) and not enabled:
        raise ValueError("ordinary issue comment edit/delete requires --ordinary-issue-comment")
    return int(enabled) + int(enabled and edited) + int(enabled and deleted)


def parse_live_delivery_record(
    record: dict[str, Any],
    *,
    owner: str,
    repository: str,
    issue_number: int,
    expected_action: str,
) -> dict[str, Any]:
    """Validate a receiver journal record for one expected live Forgejo issue event."""
    if not isinstance(record, dict):
        raise ValueError("journal record must be an object")
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("journal sequence is invalid")
    if record.get("kind") != "forgejo.webhook.v1.issues":
        raise ValueError("journal record is not an issues event")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("journal payload envelope is invalid")
    delivery_id = envelope.get("delivery_id")
    if (
        envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != "issues"
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
    ):
        raise ValueError("journal delivery envelope is invalid")
    payload = envelope.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("webhook payload is invalid")
    repository_ref = payload.get("repository")
    issue = payload.get("issue")
    if not isinstance(repository_ref, dict) or not isinstance(issue, dict):
        raise ValueError("webhook issue reference is missing")
    if repository_ref.get("full_name") != f"{owner}/{repository}":
        raise ValueError("webhook repository does not match the configured repository")
    reported_number = issue.get("number")
    if (
        not isinstance(reported_number, int)
        or isinstance(reported_number, bool)
        or reported_number != issue_number
    ):
        raise ValueError("webhook issue number does not match the configured issue")
    action = payload.get("action")
    if action != expected_action or expected_action not in {
        "opened",
        "closed",
        "reopened",
        "edited",
        "assigned",
        "unassigned",
        "label_updated",
        "label_cleared",
        "milestoned",
        "demilestoned",
    }:
        raise ValueError("webhook action does not match the expected transition")
    payload_state = issue.get("state")
    if payload_state not in {"open", "closed"}:
        raise ValueError("webhook issue state is invalid")
    result = {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": "issues",
        "action": action,
        "issue_number": issue_number,
        "payload_state": payload_state,
    }
    if expected_action in {"label_updated", "label_cleared"}:
        result["payload_labels"] = normalize_live_issue_labels(issue.get("labels"))
    if expected_action in {"assigned", "unassigned"}:
        result["payload_assignees"] = normalize_live_issue_assignees(
            issue.get("assignees")
        )
    if expected_action in {"milestoned", "demilestoned"}:
        if "milestone" not in issue:
            raise ValueError("webhook issue milestone snapshot is missing")
        milestone = normalize_live_issue_milestone(issue["milestone"])
        if (expected_action == "milestoned") != (milestone is not None):
            raise ValueError("webhook milestone snapshot does not match the transition")
        result["payload_milestone"] = milestone
    return result


def parse_live_issue_comment_delivery_record(
    record: dict[str, Any],
    *,
    owner: str,
    repository: str,
    issue_number: int,
    expected_action: str,
    expected_comment_id: int,
) -> dict[str, Any]:
    """Validate a Forgejo-emitted issue comment, excluding PR conversation comments."""
    if expected_action not in {"created", "edited", "deleted"}:
        raise ValueError("expected ordinary issue-comment action is unsupported")
    if (
        not isinstance(expected_comment_id, int)
        or isinstance(expected_comment_id, bool)
        or expected_comment_id < 1
    ):
        raise ValueError("expected ordinary issue-comment ID is invalid")
    if not isinstance(record, dict):
        raise ValueError("journal record must be an object")
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("journal sequence is invalid")
    if record.get("kind") != "forgejo.webhook.v1.issue_comment":
        raise ValueError("journal record is not an issue_comment event")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("journal payload envelope is invalid")
    delivery_id = envelope.get("delivery_id")
    if (
        envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != "issue_comment"
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
    ):
        raise ValueError("journal issue-comment delivery envelope is invalid")
    payload = envelope.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("webhook issue-comment payload is invalid")
    repository_ref = payload.get("repository")
    issue = payload.get("issue")
    comment = payload.get("comment")
    reported_number = issue.get("number") if isinstance(issue, dict) else None
    comment_id = comment.get("id") if isinstance(comment, dict) else None
    if (
        not isinstance(repository_ref, dict)
        or repository_ref.get("full_name") != f"{owner}/{repository}"
        or not isinstance(issue, dict)
        or not isinstance(reported_number, int)
        or isinstance(reported_number, bool)
        or reported_number != issue_number
        or issue_number < 1
        or payload.get("action") != expected_action
        or payload.get("is_pull") is not False
        or "pull_request" in payload
        or not isinstance(comment, dict)
        or not isinstance(comment_id, int)
        or isinstance(comment_id, bool)
        or comment_id != expected_comment_id
    ):
        raise ValueError("Forgejo payload is not the expected ordinary issue comment")
    return {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": "issue_comment",
        "action": expected_action,
        "issue_number": issue_number,
        "comment_id": comment_id,
        "is_pull": False,
    }


def validate_issue_comment_delete_readback(status: int | None, comment_id: int) -> dict[str, Any]:
    """Record deletion only when Forgejo's canonical comment route says not found."""
    if not isinstance(comment_id, int) or isinstance(comment_id, bool) or comment_id < 1:
        raise ValueError("issue-comment delete readback ID is invalid")
    if status != 404:
        raise ValueError("deleted issue comment is still present or its state is inconclusive")
    return {"comment_id": comment_id, "http_status": 404, "verdict": "absent"}


def validate_ordinary_issue_comment_reconciliation(
    exit_status: int,
    event: dict[str, Any],
    result: dict[str, Any],
    *,
    owner: str,
    repository: str,
    issue_number: int,
    expected_comment_state: str,
    expected_body: str | None,
) -> None:
    """Require the adapter to read the parent issue and canonical comment."""
    if expected_comment_state not in {"present", "absent"}:
        raise ValueError("expected ordinary issue-comment state is unsupported")
    expected_action = "deleted" if expected_comment_state == "absent" else None
    if expected_action is None and (
        not isinstance(event, dict) or event.get("action") not in {"created", "edited"}
    ):
        raise ValueError("present ordinary issue-comment state requires a create or edit event")
    if expected_action is not None and (
        not isinstance(event, dict) or event.get("action") != expected_action
    ):
        raise ValueError("absent ordinary issue-comment state requires a delete event")
    if expected_comment_state == "present" and not isinstance(expected_body, str):
        raise ValueError("present ordinary issue comment requires its canonical body")
    if expected_comment_state == "absent" and expected_body is not None:
        raise ValueError("absent ordinary issue comment cannot have an expected body")
    if (
        type(exit_status) is not int
        or exit_status != 0
        or not isinstance(result, dict)
        or result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION
        or result.get("api_version") != "forgejo-rest-v1"
        or result.get("verdict") != "current_state_read"
        or type(result.get("upstream_read_attempts")) is not int
        or result.get("upstream_read_attempts")
        != (3 if expected_comment_state == "absent" else 2)
    ):
        raise ValueError("issue-comment reconciler did not complete the expected canonical reads")
    validate_reconciliation_delivery_binding(event, result)
    if (
        type(issue_number) is not int
        or issue_number < 1
        or event.get("event") != "issue_comment"
        or type(event.get("issue_number")) is not int
        or event.get("issue_number") != issue_number
        or type(event.get("comment_id")) is not int
        or event.get("comment_id") < 1
    ):
        raise ValueError("issue-comment delivery is not bound to the expected resource")
    resource = result.get("resource")
    canonical = result.get("canonical_state")
    if (
        not isinstance(resource, dict)
        or resource.get("kind") != "issue_comment"
        or resource.get("owner") != owner
        or resource.get("repository") != repository
        or type(resource.get("issue_number")) is not int
        or resource.get("issue_number") != issue_number
        or type(resource.get("comment_id")) is not int
        or resource.get("comment_id") != event.get("comment_id")
        or not isinstance(canonical, dict)
        or canonical.get("state") not in {"open", "closed"}
        or canonical.get("html_url")
        != f"{helpers.ORIGIN}/{owner}/{repository}/issues/{issue_number}"
    ):
        raise ValueError("issue-comment canonical state is not bound to its parent issue")
    comment = canonical.get("comment")
    if (
        not isinstance(comment, dict)
        or type(comment.get("id")) is not int
        or comment.get("id") != event.get("comment_id")
        or comment.get("state") != expected_comment_state
        or comment.get("body_trust") != "untrusted_user_content"
    ):
        raise ValueError("issue-comment canonical state has the wrong identity or trust label")
    if expected_comment_state == "present":
        if (
            not isinstance(comment.get("updated_at"), str)
            or not comment["updated_at"]
            or comment.get("html_url")
            != f"{canonical['html_url']}#issuecomment-{event['comment_id']}"
            or comment.get("body") != expected_body
        ):
            raise ValueError("issue-comment canonical readback differs from the live API state")
    elif any(comment.get(field) is not None for field in ("updated_at", "html_url", "body")):
        raise ValueError("deleted issue-comment canonical state must not retain comment data")


def validate_pull_request_comment_reconciliation(
    exit_status: int,
    event: dict[str, Any],
    result: dict[str, Any],
    live_comment: dict[str, Any] | None,
    *,
    owner: str,
    repository: str,
    pull_request_number: int,
    live_comment_http_status: int | None = 200,
    batch_item: bool = False,
) -> None:
    """Bind a PR conversation-comment result to canonical Forgejo readbacks."""
    if (
        type(exit_status) is not int
        or exit_status != 0
        or not isinstance(result, dict)
        or (not batch_item and result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION)
        or (not batch_item and result.get("api_version") != "forgejo-rest-v1")
        or result.get("verdict") != "current_state_read"
        or (
            not batch_item
            and (
                type(result.get("upstream_read_attempts")) is not int
                or result.get("upstream_read_attempts")
                != (
                    3
                    if isinstance(event, dict)
                    and event.get("action") == "deleted"
                    and live_comment_http_status == 404
                    else 2
                )
            )
        )
    ):
        raise ValueError("PR-comment reconciler did not complete the expected canonical reads")
    validate_reconciliation_delivery_binding(event, result)
    if (
        type(pull_request_number) is not int
        or isinstance(pull_request_number, bool)
        or pull_request_number < 1
        or not isinstance(event, dict)
        or event.get("event") != "issue_comment"
        or event.get("action") not in {"created", "edited", "deleted"}
        or event.get("is_pull") is not True
        or type(event.get("pull_request_number")) is not int
        or event.get("pull_request_number") != pull_request_number
        or type(event.get("comment_id")) is not int
        or isinstance(event.get("comment_id"), bool)
        or event.get("comment_id") < 1
    ):
        raise ValueError("issue-comment delivery is not bound to the expected pull request")
    expected_pull_url = (
        f"{helpers.ORIGIN}/{owner}/{repository}/pulls/{pull_request_number}"
    )
    resource = result.get("resource")
    canonical = result.get("canonical_state")
    if (
        not isinstance(resource, dict)
        or resource.get("kind") != "pull_request"
        or resource.get("owner") != owner
        or resource.get("repository") != repository
        or type(resource.get("number")) is not int
        or resource.get("number") != pull_request_number
        or not isinstance(canonical, dict)
        or canonical.get("state") not in {"open", "closed"}
        or canonical.get("html_url") != expected_pull_url
    ):
        raise ValueError("PR-comment canonical state is not bound to its parent pull request")
    comment = canonical.get("comment")
    if event["action"] == "deleted":
        if (
            type(live_comment_http_status) is not int
            or live_comment_http_status != 404
            or live_comment is not None
            or not isinstance(comment, dict)
            or type(comment.get("id")) is not int
            or isinstance(comment.get("id"), bool)
            or comment.get("id") != event.get("comment_id")
            or comment.get("state") != "absent"
            or comment.get("body_trust") != "untrusted_user_content"
            or any(comment.get(field) is not None for field in ("updated_at", "html_url", "body"))
        ):
            raise ValueError("deleted PR-comment state differs from the direct API 404")
        return
    if live_comment_http_status != 200:
        raise ValueError("present PR-comment readback did not return HTTP 200")
    if (
        not isinstance(live_comment, dict)
        or type(live_comment.get("id")) is not int
        or isinstance(live_comment.get("id"), bool)
        or live_comment.get("id") != event.get("comment_id")
        or not isinstance(live_comment.get("body"), str)
        or not isinstance(live_comment.get("updated_at"), str)
        or not live_comment.get("updated_at")
        or live_comment.get("pull_request_url") != expected_pull_url
        or live_comment.get("html_url")
        != f"{expected_pull_url}#issuecomment-{event['comment_id']}"
        or not isinstance(comment, dict)
        or type(comment.get("id")) is not int
        or comment.get("id") != event.get("comment_id")
        or comment.get("state") != "present"
        or comment.get("body_trust") != "untrusted_user_content"
        or comment.get("body") != live_comment.get("body")
        or comment.get("updated_at") != live_comment.get("updated_at")
        or comment.get("html_url") != live_comment.get("html_url")
    ):
        raise ValueError("PR-comment canonical readback differs from the live API state")


def validate_deleted_pull_request_comment_inconclusive_created_event(
    exit_status: int,
    event: dict[str, Any],
    result: dict[str, Any],
) -> None:
    """A stale created event must stay inconclusive when its comment now returns 404."""
    validate_reconciliation_delivery_binding(event, result)
    if (
        type(exit_status) is not int
        or exit_status == 0
        or event.get("event") != "issue_comment"
        or event.get("action") != "created"
        or event.get("is_pull") is not True
        or result.get("verdict") != "inconclusive"
        or result.get("error", {}).get("code") != "resource_not_found"
        or result.get("error", {}).get("retryable") is not False
        or result.get("upstream_read_attempts") != 2
        or "canonical_state" in result
    ):
        raise ValueError("stale created PR-comment event was not kept inconclusive")


def validate_pull_request_comment_parent_unreadable(
    exit_status: int,
    event: dict[str, Any],
    result: dict[str, Any],
    *,
    owner: str,
    repository: str,
    pull_request_number: int,
) -> None:
    """Do not project comment state when the caller cannot read its PR parent."""
    validate_reconciliation_delivery_binding(event, result)
    error = result.get("error") if isinstance(result, dict) else None
    details = error.get("details") if isinstance(error, dict) else None
    resource = details.get("resource") if isinstance(details, dict) else None
    action = error.get("action") if isinstance(error, dict) else None
    if (
        type(exit_status) is not int
        or exit_status == 0
        or not isinstance(event, dict)
        or event.get("event") != "issue_comment"
        or event.get("action") not in {"created", "deleted"}
        or event.get("is_pull") is not True
        or type(event.get("pull_request_number")) is not int
        or isinstance(event.get("pull_request_number"), bool)
        or event.get("pull_request_number") != pull_request_number
        or type(event.get("comment_id")) is not int
        or isinstance(event.get("comment_id"), bool)
        or event.get("comment_id") < 1
        or type(pull_request_number) is not int
        or isinstance(pull_request_number, bool)
        or pull_request_number < 1
        or result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION
        or result.get("api_version") != "forgejo-rest-v1"
        or result.get("verdict") != "inconclusive"
        or type(result.get("upstream_read_attempts")) is not int
        or result.get("upstream_read_attempts") != 1
        or "canonical_state" in result
        or not isinstance(error, dict)
        or error.get("code") != "resource_not_found"
        or error.get("retryable") is not False
        or not isinstance(action, str)
        or "parent pull request" not in action
        or "repository access" not in action
        or not isinstance(details, dict)
        or details.get("upstream") != "forgejo"
        or not isinstance(resource, dict)
        or resource.get("kind") != "pull_request_comment"
        or resource.get("owner") != owner
        or resource.get("repository") != repository
        or type(resource.get("pull_request_number")) is not int
        or isinstance(resource.get("pull_request_number"), bool)
        or resource.get("pull_request_number") != pull_request_number
        or type(resource.get("comment_id")) is not int
        or isinstance(resource.get("comment_id"), bool)
        or resource.get("comment_id") != event.get("comment_id")
    ):
        raise ValueError("unreadable PR parent did not leave the comment state inconclusive")


def parse_live_push_delivery_record(
    record: dict[str, Any], *, owner: str, repository: str
) -> dict[str, Any]:
    """Validate a Forgejo-emitted branch push envelope from the journal."""
    if not isinstance(record, dict):
        raise ValueError("journal record must be an object")
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("journal sequence is invalid")
    if record.get("kind") != "forgejo.webhook.v1.push":
        raise ValueError("journal record is not a push event")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("journal payload envelope is invalid")
    delivery_id = envelope.get("delivery_id")
    if (
        envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != "push"
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
    ):
        raise ValueError("journal delivery envelope is invalid")
    payload = envelope.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("webhook push payload is invalid")
    repository_ref = payload.get("repository")
    if (
        not isinstance(repository_ref, dict)
        or repository_ref.get("full_name") != f"{owner}/{repository}"
    ):
        raise ValueError("webhook repository does not match the configured repository")
    ref = payload.get("ref")
    branch = ref.removeprefix("refs/heads/") if isinstance(ref, str) else ""
    if not isinstance(ref, str) or not ref.startswith("refs/heads/") or not branch:
        raise ValueError("webhook ref is not a branch")
    before = payload.get("before")
    after = payload.get("after")
    if (
        not isinstance(before, str)
        or not isinstance(after, str)
        or not COMMIT_SHA_RE.fullmatch(before)
        or not COMMIT_SHA_RE.fullmatch(after)
        or len(before) != len(after)
    ):
        raise ValueError("webhook push commit SHA fields are invalid")
    return {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": "push",
        "ref": ref,
        "branch": branch,
        "before": before,
        "after": after,
    }


def parse_live_delete_delivery_record(
    record: dict[str, Any], *, owner: str, repository: str
) -> dict[str, Any]:
    """Validate a Forgejo-emitted branch deletion envelope from the journal."""
    if not isinstance(record, dict):
        raise ValueError("journal record must be an object")
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("journal sequence is invalid")
    if record.get("kind") != "forgejo.webhook.v1.delete":
        raise ValueError("journal record is not a delete event")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("journal payload envelope is invalid")
    delivery_id = envelope.get("delivery_id")
    if (
        envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != "delete"
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
    ):
        raise ValueError("journal delivery envelope is invalid")
    payload = envelope.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("webhook delete payload is invalid")
    repository_ref = payload.get("repository")
    if (
        not isinstance(repository_ref, dict)
        or repository_ref.get("full_name") != f"{owner}/{repository}"
    ):
        raise ValueError("webhook repository does not match the configured repository")
    ref = payload.get("ref")
    branch = ref if isinstance(ref, str) else ""
    if (
        payload.get("ref_type") != "branch"
        or not isinstance(ref, str)
        or not branch
    ):
        raise ValueError("webhook ref is not a short branch name")
    return {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": "delete",
        "ref": ref,
        "ref_type": "branch",
        "branch": branch,
    }


def parse_live_create_delivery_record(
    record: dict[str, Any], *, owner: str, repository: str
) -> dict[str, Any]:
    """Validate a Forgejo-emitted branch creation envelope from the journal."""
    if not isinstance(record, dict):
        raise ValueError("journal record must be an object")
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("journal sequence is invalid")
    if record.get("kind") != "forgejo.webhook.v1.create":
        raise ValueError("journal record is not a create event")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("journal payload envelope is invalid")
    delivery_id = envelope.get("delivery_id")
    if (
        envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != "create"
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
    ):
        raise ValueError("journal delivery envelope is invalid")
    payload = envelope.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("webhook create payload is invalid")
    repository_ref = payload.get("repository")
    if (
        not isinstance(repository_ref, dict)
        or repository_ref.get("full_name") != f"{owner}/{repository}"
    ):
        raise ValueError("webhook repository does not match the configured repository")
    ref = payload.get("ref")
    sha = payload.get("sha")
    if (
        payload.get("ref_type") != "branch"
        or not isinstance(ref, str)
        or not ref
        or not isinstance(sha, str)
        or not COMMIT_SHA_RE.fullmatch(sha)
    ):
        raise ValueError("webhook create payload is not a valid branch and commit SHA")
    return {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": "create",
        "ref": ref,
        "ref_type": "branch",
        "branch": ref,
        "sha": sha,
    }


def parse_live_release_delivery_record(
    record: dict[str, Any],
    *,
    owner: str,
    repository: str,
    release_id: int,
    expected_action: str,
) -> dict[str, Any]:
    """Validate a Forgejo release notification while projecting only its identity."""
    allowed_actions = {"published", "updated", "deleted"}
    if (
        not isinstance(release_id, int)
        or isinstance(release_id, bool)
        or release_id < 1
        or release_id > U64_MAX
        or not isinstance(expected_action, str)
        or expected_action not in allowed_actions
    ):
        raise ValueError("expected Forgejo release identity or action is invalid")
    if not isinstance(record, dict):
        raise ValueError("journal record must be an object")
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("journal sequence is invalid")
    if record.get("kind") != "forgejo.webhook.v1.release":
        raise ValueError("journal record is not a release event")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("journal payload envelope is invalid")
    delivery_id = envelope.get("delivery_id")
    schema_version = envelope.get("schema_version")
    if (
        not isinstance(schema_version, int)
        or isinstance(schema_version, bool)
        or schema_version != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != "release"
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
    ):
        raise ValueError("journal release delivery envelope is invalid")
    payload = envelope.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("webhook release payload is invalid")
    repository_ref = payload.get("repository")
    release = payload.get("release")
    if not isinstance(repository_ref, dict) or not isinstance(release, dict):
        raise ValueError("webhook release identity is missing")
    if repository_ref.get("full_name") != f"{owner}/{repository}":
        raise ValueError("webhook release repository does not match the configured repository")
    reported_release_id = release.get("id")
    if (
        not isinstance(reported_release_id, int)
        or isinstance(reported_release_id, bool)
        or reported_release_id < 1
        or reported_release_id > U64_MAX
        or reported_release_id != release_id
    ):
        raise ValueError("webhook release ID does not match the configured release")
    action = payload.get("action")
    if action != expected_action:
        raise ValueError("webhook release action does not match the expected transition")
    return {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": "release",
        "action": action,
        "release_id": release_id,
    }


def validate_release_reconcile_readback(
    exit_status: int,
    event: dict[str, Any],
    result: dict[str, Any],
    *,
    owner: str,
    repository: str,
    release_id: int,
) -> None:
    """Accept only a verified single-delivery read of the configured current release."""
    if (
        type(exit_status) is not int
        or exit_status != 0
        or not isinstance(result, dict)
        or type(result.get("schema_version")) is not int
        or result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION
        or result.get("api_version") != "forgejo-rest-v1"
        or result.get("verdict") != "current_state_read"
        or type(result.get("upstream_read_attempts")) is not int
        or result.get("upstream_read_attempts") != 1
    ):
        raise ValueError("release reconciliation did not return one current-state read")
    validate_reconciliation_delivery_binding(event, result)
    source = result.get("journal_source")
    attestation = source.get("receiver_attestation") if isinstance(source, dict) else None
    source_hash = source.get("event_sha256") if isinstance(source, dict) else None
    if (
        not isinstance(source, dict)
        or type(source.get("sequence")) is not int
        or source.get("sequence") != event.get("sequence")
        or not isinstance(source_hash, str)
        or not re.fullmatch(r"[0-9a-f]{64}", source_hash)
        or not isinstance(attestation, dict)
        or attestation.get("status") != "verified"
    ):
        raise ValueError("release reconciliation journal attestation is missing or mismatched")
    resource = result.get("resource")
    if (
        not isinstance(resource, dict)
        or resource.get("kind") != "release"
        or resource.get("owner") != owner
        or resource.get("repository") != repository
        or type(resource.get("release_id")) is not int
        or resource.get("release_id") != release_id
    ):
        raise ValueError("release reconciliation result is not bound to the expected resource")
    canonical = result.get("canonical_state")
    release = canonical.get("release") if isinstance(canonical, dict) else None
    tag_name = release.get("tag_name") if isinstance(release, dict) else None
    if (
        not isinstance(canonical, dict)
        or canonical.get("state") != "present"
        or canonical.get("updated_at") is not None
        or not isinstance(tag_name, str)
        or not tag_name
        or any(ord(character) < 32 or ord(character) == 127 for character in tag_name)
        or type(release.get("id")) is not int
        or release.get("id") != release_id
        or not isinstance(release.get("draft"), bool)
        or not isinstance(release.get("prerelease"), bool)
        or release.get("source") != "current_forgejo_resource_api"
        or release.get("freshness") != "current_api_read"
        or release.get("trust") != "untrusted_user_content"
        or release.get("release_notes_included") is not False
        or release.get("assets_included") is not False
    ):
        raise ValueError("current release projection is invalid or contains untrusted content")
    expected_html_url = (
        f"{ORIGIN}/{quote(owner, safe='')}/{quote(repository, safe='')}"
        f"/releases/tag/{quote(tag_name, safe='')}"
    )
    if canonical.get("html_url") != expected_html_url:
        raise ValueError("current release URL is not derived from the configured repository and tag")


def reconcile_live_release_delivery(
    binary: Path,
    state: Path,
    journal: Path,
    record: dict[str, Any],
    *,
    owner: str,
    repository: str,
    release_id: int,
    expected_action: str,
    credential: str | None = None,
    target_profile: str | None = None,
    trusted_attestation_public_key: str | None = None,
) -> dict[str, Any]:
    """Bind a release journal delivery to the real local reconciler and its current API read."""
    event = parse_live_release_delivery_record(
        record,
        owner=owner,
        repository=repository,
        release_id=release_id,
        expected_action=expected_action,
    )
    exit_status, result = helpers.run_reconcile(
        binary,
        state,
        journal,
        event["delivery_id"],
        credential,
        target_profile,
        trusted_attestation_public_key=trusted_attestation_public_key,
    )
    validate_release_reconcile_readback(
        exit_status,
        event,
        result,
        owner=owner,
        repository=repository,
        release_id=release_id,
    )
    return {"event": event, "reconciliation": result}


def parse_live_pull_request_delivery_record(
    record: dict[str, Any],
    *,
    owner: str,
    repository: str,
    pull_request_number: int,
    expected_action: str,
    expected_event: str = "pull_request",
    expected_requested_reviewer_login: str | None = None,
    expected_title_from: str | None = None,
    expected_comment_id: int | None = None,
    expected_merged: bool | None = None,
    expected_base_repository_full_name: str | None = None,
    expected_head_repository_full_name: str | None = None,
) -> dict[str, Any]:
    """Validate a PR notification or comment without treating it as approval proof."""
    review_types = {
        "pull_request_approved": "pull_request_review_approved",
        "pull_request_rejected": "pull_request_review_rejected",
        "pull_request_comment": "pull_request_review_comment",
    }
    allowed_actions = {
        "pull_request": FORGEJO_V17_PULL_REQUEST_ACTIONS,
        "pull_request_comment": {"reviewed"},
        "issue_comment": {"created", "edited", "deleted"},
        **{
            event: {"reviewed"}
            for event in review_types
            if event != "pull_request_comment"
        },
    }
    if (
        not isinstance(expected_event, str)
        or not isinstance(expected_action, str)
        or expected_event not in allowed_actions
        or expected_action not in allowed_actions[expected_event]
    ):
        raise ValueError("expected Forgejo pull-request event/action pair is unsupported")
    if expected_comment_id is not None and (
        expected_event != "issue_comment"
        or not isinstance(expected_comment_id, int)
        or isinstance(expected_comment_id, bool)
        or expected_comment_id < 1
    ):
        raise ValueError("expected Forgejo comment ID is invalid")
    if expected_merged is not None and not isinstance(expected_merged, bool):
        raise ValueError("expected Forgejo merged state is invalid")
    if expected_requested_reviewer_login is not None and (
        expected_action not in {"review_requested", "review_request_removed"}
        or not isinstance(expected_requested_reviewer_login, str)
        or not expected_requested_reviewer_login
    ):
        raise ValueError("expected Forgejo requested reviewer is invalid")
    if expected_title_from is not None and (
        expected_event != "pull_request"
        or expected_action != "edited"
        or not isinstance(expected_title_from, str)
        or not expected_title_from
    ):
        raise ValueError("expected Forgejo previous pull-request title is invalid")
    if not isinstance(record, dict):
        raise ValueError("journal record must be an object")
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("journal sequence is invalid")
    if record.get("kind") != f"forgejo.webhook.v1.{expected_event}":
        raise ValueError("journal record is not the expected pull-request event")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("journal payload envelope is invalid")
    delivery_id = envelope.get("delivery_id")
    if (
        envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != expected_event
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
    ):
        raise ValueError("journal delivery envelope is invalid")
    payload = envelope.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("webhook payload is invalid")
    repository_ref = payload.get("repository")
    pull_request = payload.get("pull_request")
    if not isinstance(repository_ref, dict) or not isinstance(pull_request, dict):
        raise ValueError("webhook pull-request reference is missing")
    if repository_ref.get("full_name") != f"{owner}/{repository}":
        raise ValueError("webhook repository does not match the configured repository")
    reported_number = pull_request.get("number")
    if (
        not isinstance(reported_number, int)
        or isinstance(reported_number, bool)
        or reported_number != pull_request_number
        or pull_request_number < 1
    ):
        raise ValueError("webhook pull-request number does not match the configured request")
    action = payload.get("action")
    if action != expected_action:
        raise ValueError("webhook action does not match the expected transition")
    comment_id = None
    payload_state = pull_request.get("state")
    payload_merged = pull_request.get("merged")
    if payload_state not in {"open", "closed"} or not isinstance(payload_merged, bool):
        raise ValueError("webhook pull-request state is invalid")
    if expected_action == "reopened" and payload_state != "open":
        raise ValueError("reopened pull request state must be open")
    if expected_merged is not None and payload_merged is not expected_merged:
        raise ValueError("webhook pull-request merged state differs from the expected transition")
    merge_commit_sha = pull_request.get("merge_commit_sha")
    if expected_merged is True and (
        not isinstance(merge_commit_sha, str) or not COMMIT_SHA_RE.fullmatch(merge_commit_sha)
    ):
        raise ValueError("Forgejo merged pull-request merge commit is invalid")
    if expected_merged is False and merge_commit_sha is not None:
        raise ValueError("Forgejo unmerged pull request unexpectedly has a merge commit")
    head = pull_request.get("head")
    base = pull_request.get("base")
    head_sha = head.get("sha") if isinstance(head, dict) else None
    base_sha = base.get("sha") if isinstance(base, dict) else None
    if (
        not isinstance(head_sha, str)
        or not COMMIT_SHA_RE.fullmatch(head_sha)
        or not isinstance(base_sha, str)
        or not COMMIT_SHA_RE.fullmatch(base_sha)
    ):
        raise ValueError("webhook pull-request commit SHA is invalid")
    base_repository_full_name = None
    if expected_base_repository_full_name is not None:
        base_repository = base.get("repo") if isinstance(base, dict) else None
        base_repository_full_name = (
            base_repository.get("full_name") if isinstance(base_repository, dict) else None
        )
        if base_repository_full_name != expected_base_repository_full_name:
            raise ValueError("webhook pull-request base repository differs from the expected repository")
    head_repository_full_name = None
    if expected_head_repository_full_name is not None:
        head_repository = head.get("repo") if isinstance(head, dict) else None
        head_repository_full_name = (
            head_repository.get("full_name") if isinstance(head_repository, dict) else None
        )
        if head_repository_full_name != expected_head_repository_full_name:
            raise ValueError("webhook pull-request head repository differs from the expected repository")
    review_type = None
    payload_assignees = None
    requested_reviewer_login = None
    requested_reviewer_id = None
    payload_title = None
    changed_title_from = None
    payload_milestone = None
    has_payload_milestone = False
    if expected_event == "pull_request" and expected_action in {"assigned", "unassigned"}:
        payload_assignees = normalize_live_issue_assignees(pull_request.get("assignees"))
    if expected_event == "pull_request" and expected_action in {"milestoned", "demilestoned"}:
        if "milestone" not in pull_request:
            raise ValueError("Forgejo pull-request milestone field is missing")
        payload_milestone = normalize_live_issue_milestone(pull_request["milestone"])
        if (expected_action == "milestoned") != (payload_milestone is not None):
            raise ValueError("Forgejo pull-request milestone does not match the expected action")
        has_payload_milestone = True
    if expected_action == "reviewed" and expected_event in review_types:
        review = payload.get("review")
        review_type = review.get("type") if isinstance(review, dict) else None
        if review_type != review_types[expected_event]:
            raise ValueError("Forgejo review notification type does not match its event")
    elif expected_action in {"review_requested", "review_request_removed"}:
        requested_reviewer = payload.get("requested_reviewer")
        requested_reviewer_id = (
            requested_reviewer.get("id") if isinstance(requested_reviewer, dict) else None
        )
        requested_reviewer_login = (
            requested_reviewer.get("login") if isinstance(requested_reviewer, dict) else None
        )
        if (
            not isinstance(requested_reviewer_id, int)
            or isinstance(requested_reviewer_id, bool)
            or requested_reviewer_id < 1
            or not isinstance(requested_reviewer_login, str)
            or not requested_reviewer_login
            or (
                expected_requested_reviewer_login is not None
                and requested_reviewer_login != expected_requested_reviewer_login
            )
        ):
            raise ValueError("Forgejo requested reviewer does not match the expected reviewer")
    elif expected_event == "pull_request" and expected_action == "edited":
        changes = payload.get("changes")
        title_change = changes.get("title") if isinstance(changes, dict) else None
        payload_title = pull_request.get("title")
        changed_title_from = title_change.get("from") if isinstance(title_change, dict) else None
        if (
            not isinstance(payload_title, str)
            or not payload_title
            or not isinstance(changed_title_from, str)
            or not changed_title_from
            or (
                expected_title_from is not None
                and changed_title_from != expected_title_from
            )
        ):
            raise ValueError("Forgejo pull-request title edit differs from the expected transition")
    elif expected_event == "issue_comment":
        issue = payload.get("issue")
        comment = payload.get("comment")
        is_pull = payload.get("is_pull")
        if (
            not isinstance(issue, dict)
            or issue.get("number") != pull_request_number
            or not isinstance(comment, dict)
            or is_pull is not True
        ):
            raise ValueError("Forgejo pull-request conversation comment payload is invalid")
        comment_id = comment.get("id")
        if (
            not isinstance(comment_id, int)
            or isinstance(comment_id, bool)
            or comment_id < 1
            or (expected_comment_id is not None and comment_id != expected_comment_id)
        ):
            raise ValueError("Forgejo pull-request comment ID does not match the expected comment ID")
    result = {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": expected_event,
        "action": action,
        "pull_request_number": pull_request_number,
        "payload_state": payload_state,
        "payload_merged": payload_merged,
        "head_sha": head_sha,
        "base_sha": base_sha,
    }
    if expected_base_repository_full_name is not None:
        result["base_repository_full_name"] = base_repository_full_name
    if expected_head_repository_full_name is not None:
        result["head_repository_full_name"] = head_repository_full_name
    if expected_merged is True:
        result["merge_commit_sha"] = merge_commit_sha
    if review_type is not None:
        result["review_type"] = review_type
    if expected_event == "pull_request" and expected_action in {"assigned", "unassigned"}:
        result["payload_assignees"] = payload_assignees
    if has_payload_milestone:
        result["payload_milestone"] = payload_milestone
    if requested_reviewer_login is not None:
        result["requested_reviewer_login"] = requested_reviewer_login
        result["requested_reviewer_id"] = requested_reviewer_id
    if expected_event == "pull_request" and expected_action == "edited":
        result["payload_title"] = payload_title
        result["changed_title_from"] = changed_title_from
    if comment_id is not None:
        result["comment_id"] = comment_id
        result["is_pull"] = True
    return result


def normalize_live_work_item_labels(
    value: Any, *, resource_name: str
) -> list[dict[str, Any]]:
    """Project the bounded id/name subset from an untrusted live work-item payload."""
    if not isinstance(value, list) or len(value) > 256:
        raise ValueError(f"live {resource_name} labels must be a bounded array")
    normalized: list[dict[str, Any]] = []
    seen_ids: set[int] = set()
    for label in value:
        label_id = label.get("id") if isinstance(label, dict) else None
        name = label.get("name") if isinstance(label, dict) else None
        if (
            type(label_id) is not int
            or label_id < 1
            or label_id in seen_ids
            or not isinstance(name, str)
            or not name
            or len(name.encode("utf-8")) > 255
            or any(ord(character) < 32 or ord(character) == 127 for character in name)
        ):
            raise ValueError(f"live {resource_name} label identity is malformed")
        seen_ids.add(label_id)
        normalized.append({"id": label_id, "name": name})
    normalized.sort(key=lambda label: label["id"])
    return normalized


def normalize_live_pull_request_labels(value: Any) -> list[dict[str, Any]]:
    return normalize_live_work_item_labels(value, resource_name="pull-request")


def normalize_live_issue_labels(value: Any) -> list[dict[str, Any]]:
    return normalize_live_work_item_labels(value, resource_name="issue")


def normalize_live_issue_assignees(value: Any) -> list[dict[str, Any]] | None:
    """Validate webhook or PR assignees; null remains unknown."""
    if value is None:
        return None
    if not isinstance(value, list) or len(value) > 256:
        raise ValueError("live issue assignees must be a bounded array")
    normalized: list[dict[str, Any]] = []
    seen_ids: set[int] = set()
    for assignee in value:
        assignee_id = assignee.get("id") if isinstance(assignee, dict) else None
        login = assignee.get("login") if isinstance(assignee, dict) else None
        if (
            type(assignee_id) is not int
            or assignee_id < 1
            or assignee_id in seen_ids
            or not isinstance(login, str)
            or not login.strip()
            or len(login.encode("utf-8")) > 255
            or any(ord(char) < 32 or 0x7F <= ord(char) <= 0x9F for char in login)
        ):
            raise ValueError("live issue assignee identity is invalid")
        seen_ids.add(assignee_id)
        normalized.append({"id": assignee_id, "login": login})
    normalized.sort(key=lambda assignee: assignee["id"])
    return normalized


def normalize_current_issue_api_assignees(
    issue: Any,
) -> list[dict[str, Any]] | None:
    """Project Forgejo v17 issue API null as empty only when the field exists."""
    if not isinstance(issue, dict) or "assignees" not in issue:
        return None
    normalized = normalize_live_issue_assignees(issue["assignees"])
    return [] if normalized is None else normalized


def normalize_live_issue_milestone(value: Any) -> dict[str, Any] | None:
    """Project only the bounded identity fields needed to compare milestone state."""
    if value is None:
        return None
    milestone_id = value.get("id") if isinstance(value, dict) else None
    title = value.get("title") if isinstance(value, dict) else None
    state = value.get("state") if isinstance(value, dict) else None
    if (
        type(milestone_id) is not int
        or milestone_id < 1
        or not isinstance(title, str)
        or not title.strip()
        or len(title.encode("utf-8")) > 1024
        or any(ord(character) < 32 or ord(character) == 127 for character in title)
        or not isinstance(state, str)
        or state not in {"open", "closed"}
    ):
        raise ValueError("live issue milestone identity is malformed")
    return {"id": milestone_id, "title": title, "state": state}


def validate_live_issue_milestone_clear_snapshot(
    *,
    clear_payload_milestone: dict[str, Any] | None,
    update_payload_milestone: dict[str, Any],
    api_milestone_after_clear: dict[str, Any] | None,
    expected_milestone_id: int,
    expected_milestone_title: str,
) -> str:
    """Require API-confirmed unassignment while retaining the transition snapshots."""
    if update_payload_milestone != {
        "id": expected_milestone_id,
        "title": expected_milestone_title,
        "state": "open",
    }:
        raise ValueError("issue milestoned event omitted the disposable milestone")
    if clear_payload_milestone not in (None, update_payload_milestone):
        raise ValueError("issue demilestoned snapshot differs from the prior assignment snapshot")
    if api_milestone_after_clear is not None:
        raise ValueError("current Forgejo issue API still reports a milestone after the clear")
    return "known_unassigned"


def validate_live_pull_request_milestone_clear_snapshot(
    *,
    clear_payload_milestone: dict[str, Any] | None,
    update_payload_milestone: dict[str, Any],
    api_milestone_after_clear: dict[str, Any] | None,
    expected_milestone_id: int,
    expected_milestone_title: str,
) -> str:
    """Bind a native PR clear snapshot to its assignment and current API readback."""
    if update_payload_milestone != {
        "id": expected_milestone_id,
        "title": expected_milestone_title,
        "state": "open",
    }:
        raise ValueError("pull-request milestoned event omitted the disposable milestone")
    if clear_payload_milestone not in (None, update_payload_milestone):
        raise ValueError("pull-request demilestoned snapshot differs from the prior assignment")
    if api_milestone_after_clear is not None:
        raise ValueError("current Forgejo pull-request API still reports a milestone")
    return "known_unassigned"


def is_current_unassigned_milestone_projection(value: Any) -> bool:
    return value == {
        "observation": "unassigned",
        "value": None,
        "source": "current_forgejo_resource_api",
        "freshness": "current_api_read",
        "trust": "untrusted_user_content",
    }


def is_current_empty_assignee_projection(value: Any) -> bool:
    return value == {
        "items": [],
        "source": "current_forgejo_resource_api",
        "freshness": "current_api_read",
        "trust": "untrusted_forgejo_identity",
    }


def validate_live_issue_assignee_clear_snapshot(
    *,
    clear_payload_assignees: list[dict[str, Any]] | None,
    update_payload_assignees: list[dict[str, Any]],
    api_assignees_after_clear: list[dict[str, Any]] | None,
    expected_assignee: dict[str, Any],
) -> str:
    """Keep the transition snapshot separate from the canonical post-clear API state."""
    if expected_assignee not in update_payload_assignees:
        raise ValueError("issue assigned event omitted the disposable assignee")
    if clear_payload_assignees not in (None, [], update_payload_assignees):
        raise ValueError("issue unassigned snapshot differs from the prior assignment snapshot")
    if api_assignees_after_clear is None:
        return "unknown_null_field"
    if api_assignees_after_clear == []:
        return "known_empty"
    if api_assignees_after_clear:
        raise ValueError("current Forgejo issue API still reports assignees after the clear")
    raise ValueError("current Forgejo issue API assignee state is invalid")


def validate_live_pull_request_label_clear_snapshot(
    *,
    clear_payload_labels: list[dict[str, Any]],
    update_payload_labels: list[dict[str, Any]],
    api_labels_after_clear: list[dict[str, Any]],
    expected_label_id: int,
    expected_label_name: str,
) -> None:
    """Keep the webhook snapshot separate from the canonical post-clear API state."""
    expected_label = {"id": expected_label_id, "name": expected_label_name}
    if expected_label not in update_payload_labels:
        raise ValueError("label_updated event omitted the disposable label")
    if clear_payload_labels != update_payload_labels:
        raise ValueError("label_cleared event snapshot differs from the prior update snapshot")
    if api_labels_after_clear:
        raise ValueError("current Forgejo API still reports labels after the clear")


def validate_live_issue_label_clear_snapshot(
    *,
    clear_payload_labels: list[dict[str, Any]],
    update_payload_labels: list[dict[str, Any]],
    api_labels_after_clear: list[dict[str, Any]],
    expected_label_id: int,
    expected_label_name: str,
) -> None:
    """Accept either a stale transition snapshot or an empty issue clear payload."""
    expected_label = {"id": expected_label_id, "name": expected_label_name}
    if expected_label not in update_payload_labels:
        raise ValueError("issue label_updated event omitted the disposable label")
    if clear_payload_labels not in ([], update_payload_labels):
        raise ValueError("issue label_cleared snapshot does not match the prior update snapshot")
    if api_labels_after_clear:
        raise ValueError("current Forgejo issue API still reports labels after the clear")


def is_current_empty_label_projection(value: Any) -> bool:
    """Require the adapter's explicitly sourced empty current-API projection."""
    return (
        isinstance(value, dict)
        and set(value) == {"items", "source", "freshness", "trust"}
        and value.get("items") == []
        and value.get("source") == "current_forgejo_resource_api"
        and value.get("freshness") == "current_api_read"
        and value.get("trust") == "untrusted_user_content"
    )


def read_journal_records(path: Path) -> list[dict[str, Any]]:
    try:
        size = path.stat().st_size
    except FileNotFoundError:
        return []
    if size > MAX_JOURNAL_BYTES:
        raise RuntimeError("local webhook journal exceeded the E2E read limit")
    raw = path.read_bytes()
    if raw and not raw.endswith(b"\n"):
        raise RuntimeError("local webhook journal ended with a partial record")
    records: list[dict[str, Any]] = []
    for line_number, line in enumerate(raw.splitlines(), 1):
        if not line:
            raise RuntimeError(f"local webhook journal contains an empty record at line {line_number}")
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            raise RuntimeError(f"local webhook journal has invalid JSON at line {line_number}") from None
        if not isinstance(value, dict):
            raise RuntimeError(f"local webhook journal record {line_number} is not an object")
        records.append(value)
    return records


def wait_for_journal_count(
    path: Path,
    expected: int,
    timeout: float = DEFAULT_WEBHOOK_JOURNAL_WAIT_SECONDS,
) -> list[dict[str, Any]]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        records = read_journal_records(path)
        if len(records) >= expected:
            return records
        time.sleep(0.1)
    raise RuntimeError("Forgejo did not deliver the expected webhook event to the receiver")


def validate_hook_task_bindings(
    tasks: list[dict[str, Any]], events: list[dict[str, Any]]
) -> None:
    """Bind Forgejo task UUIDs to accepted receiver events independent of delivery order."""
    if not tasks or len(tasks) != len(events):
        raise ValueError("Forgejo task and receiver event counts do not match")
    task_types = {
        "issues": {"issues", "issue_assign", "issue_label", "issue_milestone"},
        "release": {"release"},
        "create": {"create"},
        "push": {"push"},
        "delete": {"delete"},
        "pull_request": {
            "pull_request", "pull_request_assign", "pull_request_label",
            "pull_request_milestone", "pull_request_sync", "pull_request_review_request",
        },
        "pull_request_approved": {"pull_request_review_approved"},
        "pull_request_rejected": {"pull_request_review_rejected"},
        "pull_request_comment": {"pull_request_review_comment"},
        "issue_comment": {"issue_comment", "pull_request_comment"},
    }
    by_delivery = {
        event.get("delivery_id"): event for event in events if isinstance(event, dict)
    }
    if len(by_delivery) != len(events) or any(not isinstance(key, str) for key in by_delivery):
        raise ValueError("receiver journal contains duplicate or invalid delivery IDs")
    seen: set[str] = set()
    for task in tasks:
        if (
            not isinstance(task, dict)
            or not isinstance(task.get("uuid"), str)
            or task.get("uuid") in seen
        ):
            raise ValueError("Forgejo delivery tasks contain duplicate or invalid UUIDs")
        event = by_delivery.get(task["uuid"])
        event_name = event.get("event") if isinstance(event, dict) else None
        if (
            event_name not in task_types
            or task.get("event_type") not in task_types.get(event_name, set())
            or task.get("is_delivered") is not True
            or task.get("is_succeed") is not True
            or task.get("response_status") != 202
        ):
            raise ValueError("Forgejo delivery task does not match its receiver journal event")
        seen.add(task["uuid"])


def validate_reconciliation_delivery_binding(
    event: dict[str, Any], result: dict[str, Any]
) -> None:
    """Ensure a reconciliation result belongs to the delivery that was requested."""
    delivery_id = event.get("delivery_id") if isinstance(event, dict) else None
    result_delivery_id = result.get("delivery_id") if isinstance(result, dict) else None
    if not isinstance(delivery_id, str) or not delivery_id or result_delivery_id != delivery_id:
        raise ValueError("reconciliation result delivery ID does not match the requested event")


def validate_work_item_content_projection(
    result: dict[str, Any],
    *,
    expected_title: str,
    expected_body: str | None,
    expected_source: str,
    batch_item: bool = False,
) -> None:
    """Require bounded title/body from the current canonical resource API."""
    canonical = result.get("canonical_state") if isinstance(result, dict) else None
    content = canonical.get("work_item_content") if isinstance(canonical, dict) else None
    if (
        not isinstance(result, dict)
        or (not batch_item and result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION)
        or not isinstance(content, dict)
        or set(content) != {"title", "body", "source", "trust"}
        or not isinstance(content.get("title"), str)
        or len(content["title"].encode("utf-8")) > 1024
        or (
            content.get("body") is not None
            and (
                not isinstance(content.get("body"), str)
                or len(content["body"].encode("utf-8")) > 65_536
            )
        )
        or content.get("title") != expected_title
        or content.get("body") != expected_body
        or content.get("source") != expected_source
        or content.get("trust") != "untrusted_user_content"
    ):
        raise ValueError("reconciliation did not project the current bounded work-item content")


def parse_loopback_connect_trace(trace: str, *, expected_host: str, expected_port: int) -> dict[str, Any]:
    """Count IPv4 connect syscalls without recording argv, headers, or payloads."""
    if not isinstance(trace, str) or len(trace) > 64 * 1024:
        raise ValueError("batch connection trace is invalid or exceeds the local evidence limit")
    if expected_host != "127.0.0.1" or type(expected_port) is not int or expected_port < 1:
        raise ValueError("batch connection observer target is not the configured loopback origin")

    attempts: list[str] = []
    unix_domain_attempts = 0
    for line in trace.splitlines():
        if "connect(" not in line:
            continue
        if "sa_family=AF_UNIX" in line:
            unix_domain_attempts += 1
            continue
        match = re.search(
            r'connect\(\d+, \{sa_family=AF_INET, sin_port=htons\((\d+)\), '
            r'sin_addr=inet_addr\("([^\"]+)"\)\}, \d+\) = (.+)$',
            line,
        )
        if match is None:
            raise ValueError("batch connection trace contains an unrecognized network connect")
        if match.group(1) != str(expected_port) or match.group(2) != expected_host:
            raise ValueError("batch reconciliation attempted a non-fixture network destination")
        syscall_result = match.group(3)
        if syscall_result != "0" and not syscall_result.startswith("-1 EINPROGRESS"):
            raise ValueError("batch reconciliation did not complete its loopback connect")
        attempts.append(syscall_result)

    if not attempts:
        raise ValueError("batch reconciliation did not connect to the configured loopback fixture")
    observation = {
        "observer": "strace syscall trace restricted to connect",
        "target": f"{expected_host}:{expected_port}",
        "network_connect_attempts": len(attempts),
        "connect_results": attempts,
        "unix_domain_connect_attempts_ignored": unix_domain_attempts,
        "argv_traced": False,
        "request_headers_or_bodies_traced": False,
    }
    if len(attempts) == 1:
        # Keep the original singular field for consumers of earlier evidence.
        observation["connect_result"] = attempts[0]
    return observation


def run_reconcile_batch(
    binary: Path,
    state: Path,
    journal: Path,
    delivery_ids: list[str],
    credential: str,
    trusted_attestation_public_key: str,
    *,
    trace_path: Path | None = None,
    strace_binary: Path | None = None,
) -> tuple[int, dict[str, Any], bytes]:
    if (
        len(delivery_ids) < 2
        or len(delivery_ids) > 100
        or len(set(delivery_ids)) != len(delivery_ids)
        or any(not isinstance(value, str) or not DELIVERY_ID_RE.fullmatch(value) for value in delivery_ids)
    ):
        raise ValueError("batch requires 2-100 unique valid delivery IDs")
    if (
        len(trusted_attestation_public_key) != 64
        or any(character not in "0123456789abcdef" for character in trusted_attestation_public_key)
    ):
        raise ValueError("trusted receiver attestation key is not canonical lower-case Ed25519 hex")
    credential_bytes = credential.encode("utf-8")
    read_fd, write_fd = os.pipe()
    try:
        view = memoryview(credential_bytes)
        while view:
            written = os.write(write_fd, view)
            if written <= 0:
                raise OSError("short write to private reconciliation credential pipe")
            view = view[written:]
    finally:
        os.close(write_fd)
    try:
        command = [
            str(binary),
            "--trusted-webhook-attestation-public-key-hex",
            trusted_attestation_public_key,
            "--host-credential-fd",
            str(read_fd),
            str(journal),
            *delivery_ids,
        ]
        if trace_path is not None:
            if trace_path.exists() or trace_path.is_symlink():
                raise RuntimeError("refusing to overwrite an existing batch connection trace")
            if strace_binary is None or not strace_binary.is_file():
                raise RuntimeError("strace is required for the requested batch connection observation")
            command = [
                str(strace_binary),
                "-f",
                "-qq",
                "-e",
                "trace=connect",
                "-o",
                str(trace_path),
                "--",
                *command,
            ]
        completed = subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=helpers.private_command_environment(state),
            pass_fds=(read_fd,),
            timeout=20,
            check=False,
        )
        remaining_credential = bytearray()
        while True:
            chunk = os.read(read_fd, len(credential_bytes) + 1)
            if not chunk:
                break
            remaining_credential.extend(chunk)
    finally:
        os.close(read_fd)
    if credential_bytes in completed.stdout or credential_bytes in completed.stderr:
        raise RuntimeError("batch reconciliation CLI exposed its private credential")
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError:
        raise RuntimeError("batch reconciliation CLI did not return structured JSON") from None
    if not isinstance(value, dict):
        raise RuntimeError("batch reconciliation CLI returned a non-object result")
    return completed.returncode, value, bytes(remaining_credential)


def validate_pull_request_reconcile_batch(
    exit_status: int,
    requested_events: list[dict[str, Any]],
    batch: dict[str, Any],
    *,
    owner: str,
    repository: str,
    pull_request_number: int,
    expected_requested_reviewers: list[dict[str, Any]] | None = None,
    expected_requested_reviewer_teams: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    normalized_expected_reviewers: list[dict[str, Any]] | None = None
    if expected_requested_reviewers is not None:
        if len(expected_requested_reviewers) > 1024:
            raise ValueError("expected requested-reviewer list exceeds the projection limit")
        normalized_expected_reviewers = []
        reviewer_ids: set[int] = set()
        for reviewer in expected_requested_reviewers:
            reviewer_id = reviewer.get("id") if isinstance(reviewer, dict) else None
            login = reviewer.get("login") if isinstance(reviewer, dict) else None
            if (
                type(reviewer_id) is not int
                or reviewer_id < 1
                or reviewer_id in reviewer_ids
                or not isinstance(login, str)
                or not login
                or any(ord(character) < 32 or ord(character) == 127 for character in login)
            ):
                raise ValueError("expected requested-reviewer identity is malformed")
            reviewer_ids.add(reviewer_id)
            normalized_expected_reviewers.append({"id": reviewer_id, "login": login})
        normalized_expected_reviewers.sort(key=lambda reviewer: reviewer["id"])
    normalized_expected_reviewer_teams: list[dict[str, Any]] | None = None
    if expected_requested_reviewer_teams is not None:
        if len(expected_requested_reviewer_teams) > 1024:
            raise ValueError("expected requested-reviewer-team list exceeds the projection limit")
        normalized_expected_reviewer_teams = []
        team_ids: set[int] = set()
        for team in expected_requested_reviewer_teams:
            team_id = team.get("id") if isinstance(team, dict) else None
            name = team.get("name") if isinstance(team, dict) else None
            if (
                type(team_id) is not int
                or team_id < 1
                or team_id in team_ids
                or not isinstance(name, str)
                or not name
                or len(name) > 255
                or any(ord(character) < 32 or ord(character) == 127 for character in name)
            ):
                raise ValueError("expected requested-reviewer-team identity is malformed")
            team_ids.add(team_id)
            normalized_expected_reviewer_teams.append({"id": team_id, "name": name})
        normalized_expected_reviewer_teams.sort(key=lambda team: team["id"])
    expected_upstream_reads = 1
    seen_comment_reads: set[tuple[int, bool]] = set()
    if isinstance(requested_events, list):
        for event in requested_events:
            if not isinstance(event, dict) or event.get("event") != "issue_comment":
                continue
            comment_id = event.get("comment_id")
            action = event.get("action")
            if type(comment_id) is int and comment_id > 0 and action in {
                "created",
                "edited",
                "deleted",
            }:
                deleted = action == "deleted"
                key = (comment_id, deleted)
                if key not in seen_comment_reads:
                    seen_comment_reads.add(key)
                    expected_upstream_reads += 1
                if deleted:
                    # A deleted-comment 404 requires a fresh parent read so
                    # it cannot be mistaken for a permissions failure.
                    expected_upstream_reads += 1
    if type(exit_status) is not int or exit_status != 0:
        raise ValueError("pull-request reconciliation batch did not exit successfully")
    if (
        not isinstance(batch, dict)
        or batch.get("schema_version") != RECONCILE_BATCH_SCHEMA_VERSION
        or batch.get("api_version") != "forgejo-rest-v1"
        or batch.get("verdict") != "current_state_read"
        or type(batch.get("upstream_read_attempts")) is not int
        or batch.get("upstream_read_attempts") != expected_upstream_reads
    ):
        if expected_upstream_reads == 1:
            raise ValueError("pull-request reconciliation batch did not report one current-state read")
        raise ValueError(
            "pull-request reconciliation batch did not report the expected "
            f"{expected_upstream_reads} upstream reads"
        )
    results = batch.get("results")
    if (
        not isinstance(requested_events, list)
        or len(requested_events) < 2
        or not isinstance(results, list)
        or len(results) != len(requested_events)
        or type(pull_request_number) is not int
        or pull_request_number < 1
    ):
        raise ValueError("pull-request reconciliation batch selection or result count is invalid")
    seen: set[str] = set()
    checked: list[dict[str, Any]] = []
    for event, result in zip(requested_events, results, strict=True):
        delivery_id = event.get("delivery_id") if isinstance(event, dict) else None
        if (
            not isinstance(delivery_id, str)
            or not DELIVERY_ID_RE.fullmatch(delivery_id)
            or delivery_id in seen
            or type(event.get("pull_request_number")) is not int
            or event.get("pull_request_number") != pull_request_number
        ):
            raise ValueError("pull-request batch selection contains duplicate or foreign deliveries")
        seen.add(delivery_id)
        validate_reconciliation_delivery_binding(event, result)
        resource = result.get("resource") if isinstance(result, dict) else None
        canonical = result.get("canonical_state") if isinstance(result, dict) else None
        if (
            not isinstance(result, dict)
            or result.get("verdict") != "current_state_read"
            or not isinstance(resource, dict)
            or resource.get("kind") != "pull_request"
            or type(resource.get("number")) is not int
            or resource.get("number") != pull_request_number
            or resource.get("owner") != owner
            or resource.get("repository") != repository
            or not isinstance(canonical, dict)
            or canonical.get("state") not in {"open", "closed"}
            or not isinstance(canonical.get("merged"), bool)
            or not isinstance(canonical.get("head_sha"), str)
            or not COMMIT_SHA_RE.fullmatch(canonical["head_sha"])
            or not isinstance(canonical.get("base_sha"), str)
            or not COMMIT_SHA_RE.fullmatch(canonical["base_sha"])
            or (
                normalized_expected_reviewers is not None
                and canonical.get("requested_reviewers") != normalized_expected_reviewers
            )
            or (
                normalized_expected_reviewer_teams is not None
                and canonical.get("requested_reviewers_teams")
                != normalized_expected_reviewer_teams
            )
        ):
            raise ValueError(
                "pull-request batch item is not a bound canonical PR state"
                if normalized_expected_reviewers is None
                and normalized_expected_reviewer_teams is None
                else "pull-request batch item differs from the current reviewer readback"
            )
        checked.append(result)
    return checked


def normalize_requested_reviewer_teams_readback(
    pull_request: Any,
) -> list[dict[str, Any]] | None:
    if not isinstance(pull_request, dict):
        raise RuntimeError("Forgejo PR readback returned a non-object")
    raw_teams = pull_request.get("requested_reviewers_teams")
    if raw_teams is None:
        return None
    if not isinstance(raw_teams, list) or len(raw_teams) > 1024:
        raise RuntimeError("Forgejo PR readback returned malformed requested reviewer teams")
    normalized: list[dict[str, Any]] = []
    team_ids: set[int] = set()
    for team in raw_teams:
        team_id = team.get("id") if isinstance(team, dict) else None
        team_name = team.get("name") if isinstance(team, dict) else None
        if (
            type(team_id) is not int
            or team_id < 1
            or team_id in team_ids
            or not isinstance(team_name, str)
            or not team_name
            or len(team_name) > 255
            or any(ord(character) < 32 or ord(character) == 127 for character in team_name)
        ):
            raise RuntimeError("Forgejo PR readback returned malformed requested reviewer teams")
        team_ids.add(team_id)
        normalized.append({"id": team_id, "name": team_name})
    normalized.sort(key=lambda team: team["id"])
    return normalized


def select_duplicate_replay_event(
    fallback_event: dict[str, Any], preferred_events: list[dict[str, Any]]
) -> dict[str, Any]:
    """Use a scenario-specific event for duplicate replay, preserving the common fallback."""
    if not isinstance(fallback_event, dict) or not isinstance(preferred_events, list):
        raise ValueError("duplicate replay event selection is invalid")
    if any(not isinstance(event, dict) for event in preferred_events):
        raise ValueError("duplicate replay event list contains a malformed event")
    event = preferred_events[0] if preferred_events else fallback_event
    if (
        not isinstance(event.get("delivery_id"), str)
        or not event.get("delivery_id")
        or not isinstance(event.get("event"), str)
        or type(event.get("sequence")) is not int
        or event["sequence"] < 1
    ):
        raise ValueError("duplicate replay event lacks a valid delivery identity")
    return event


def validate_duplicate_delivery_replay(
    event: dict[str, Any],
    status: int,
    response: dict[str, Any],
    records_before: list[dict[str, Any]],
    records_after: list[dict[str, Any]],
) -> None:
    """Require a redelivery to bind to the same event without extending the journal."""
    delivery_id = event.get("delivery_id") if isinstance(event, dict) else None
    sequence = event.get("sequence") if isinstance(event, dict) else None
    if (
        not isinstance(delivery_id, str)
        or not delivery_id
        or not isinstance(sequence, int)
        or isinstance(sequence, bool)
        or not isinstance(response, dict)
        or type(status) is not int
        or status != 202
        or response.get("delivery_id") != delivery_id
    ):
        raise ValueError("duplicate replay response delivery ID or HTTP status is invalid")
    if (
        response.get("accepted") is not True
        or response.get("duplicate") is not True
        or response.get("reconcile_required") is not True
        or type(response.get("sequence")) is not int
        or response.get("sequence") != sequence
    ):
        raise ValueError("receiver did not confirm the matching duplicate delivery")
    if records_before != records_after:
        raise ValueError("receiver journal changed during duplicate delivery replay")


def validate_journal_snapshot_unchanged(before: bytes, after: bytes) -> None:
    """Reject byte-level journal rewrites even when their parsed JSON is equal."""
    if not isinstance(before, bytes) or not isinstance(after, bytes) or before != after:
        raise ValueError("receiver journal bytes changed during duplicate delivery replay")


def kill_process_with_sigkill(process: subprocess.Popen[bytes] | None) -> dict[str, Any]:
    """Send SIGKILL to a running process and report its observed termination signal."""
    if process is None:
        return {
            "started": False,
            "stopped": True,
            "returncode": None,
            "stop_action": "not_started",
            "stop_signal": None,
        }

    action = "already_exited"
    if process.poll() is None:
        action = "sigkill"
        try:
            process.kill()
        except ProcessLookupError:
            action = "exited_during_sigkill"
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            raise RuntimeError("receiver did not exit after SIGKILL") from None

    returncode = process.returncode
    return {
        "started": True,
        "stopped": process.poll() is not None,
        "returncode": returncode,
        "stop_action": action,
        "stop_signal": "SIGKILL" if returncode == -signal.SIGKILL else None,
    }


def send_redelivered_webhook(
    receiver_address: str,
    secret: bytes,
    delivery_id: str,
    event_name: str,
    payload: dict[str, Any],
) -> tuple[int, dict[str, Any]]:
    """Resend one captured journal payload to the loopback receiver with its HMAC."""
    host, separator, port_text = receiver_address.rpartition(":")
    if (
        separator != ":"
        or host != "127.0.0.1"
        or not port_text.isascii()
        or not port_text.isdecimal()
        or not 1 <= int(port_text) <= 65535
        or not isinstance(secret, bytes)
        or not secret
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or not isinstance(event_name, str)
        or event_name not in HOOK_EVENT_GROUPS
        or not isinstance(payload, dict)
    ):
        raise ValueError("redelivery target or webhook event is invalid")
    try:
        body = json.dumps(
            payload,
            ensure_ascii=False,
            separators=(",", ":"),
            allow_nan=False,
        ).encode("utf-8")
    except (TypeError, ValueError):
        raise ValueError("redelivery payload is not valid JSON") from None
    digest = hmac.new(secret, body, hashlib.sha256).hexdigest()
    connection = http.client.HTTPConnection(host, int(port_text), timeout=5)
    try:
        connection.request(
            "POST",
            "/forgejo/webhook",
            body=body,
            headers={
                "Content-Type": "application/json",
                "X-Forgejo-Delivery": delivery_id,
                "X-Forgejo-Event": event_name,
                "X-Forgejo-Signature": digest,
            },
        )
        response = connection.getresponse()
        response_body = response.read(64 * 1024 + 1)
        if len(response_body) > 64 * 1024:
            raise RuntimeError("receiver redelivery response exceeded the local limit")
        try:
            value = json.loads(response_body) if response_body else {}
        except json.JSONDecodeError:
            raise RuntimeError("receiver redelivery response was not JSON") from None
        if not isinstance(value, dict):
            raise RuntimeError("receiver redelivery response was not an object")
        return response.status, value
    except (OSError, http.client.HTTPException):
        raise RuntimeError("receiver redelivery request failed") from None
    finally:
        connection.close()


def hook_task_readback(database: Path, hook_id: int) -> list[dict[str, Any]]:
    if not database.is_file():
        raise RuntimeError("local Forgejo SQLite database is missing")
    uri = database.resolve().as_uri() + "?mode=ro"
    try:
        with sqlite3.connect(uri, uri=True, timeout=2.0) as db:
            rows = db.execute(
                "SELECT id, uuid, event_type, is_delivered, is_succeed, response_content "
                "FROM hook_task WHERE hook_id = ? ORDER BY id ASC",
                (hook_id,),
            ).fetchall()
    except sqlite3.Error:
        raise RuntimeError("read-only Forgejo webhook-task readback failed") from None
    tasks: list[dict[str, Any]] = []
    for task_id, delivery_uuid, event_type, delivered, succeeded, response_content in rows:
        try:
            response = json.loads(response_content) if response_content else {}
        except json.JSONDecodeError:
            response = {}
        status = response.get("status") if isinstance(response, dict) else None
        tasks.append(
            {
                "task_id": task_id,
                "uuid": delivery_uuid,
                "event_type": event_type,
                "is_delivered": delivered == 1,
                "is_succeed": succeeded == 1,
                "response_status": status if isinstance(status, int) else None,
            }
        )
    return tasks


def wait_for_hook_tasks(database: Path, hook_id: int, expected: int, timeout: float = 10.0) -> list[dict[str, Any]]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        tasks = hook_task_readback(database, hook_id)
        if len(tasks) >= expected and all(
            task["is_delivered"] and task["is_succeed"] and task["response_status"] == 202
            for task in tasks[:expected]
        ):
            return tasks
        time.sleep(0.1)
    raise RuntimeError("Forgejo did not record successful HTTP 202 webhook deliveries")


def hook_deactivation_patch(
    event_selectors: tuple[str, ...] = HOOK_EVENT_SELECTORS,
) -> dict[str, Any]:
    """Disable the hook without letting Forgejo reset omitted event fields."""
    effective_hook_events(event_selectors)
    return {"active": False, "events": list(event_selectors)}


def hook_readback_record(
    hook_id: int | None,
    api_readback: dict[str, Any] | None,
    expected_target: str | None,
    expected_event_selectors: tuple[str, ...] = HOOK_EVENT_SELECTORS,
) -> dict[str, Any]:
    readback = api_readback if isinstance(api_readback, dict) else {}
    readback_id = readback.get("id")
    if not isinstance(readback_id, int) or isinstance(readback_id, bool):
        readback_id = None
    config = readback.get("config")
    config = config if isinstance(config, dict) else {}
    raw_events = readback.get("events")
    events_readback_valid = isinstance(raw_events, list) and all(
        isinstance(event, str) for event in raw_events
    )
    events = (
        [event for event in raw_events if isinstance(event, str)]
        if isinstance(raw_events, list)
        else None
    )
    hook_identity_matches = hook_id is not None and readback_id == hook_id
    readback_active = readback.get("active")
    active_after_run = (
        readback_active if hook_identity_matches and isinstance(readback_active, bool) else None
    )
    deactivation_confirmed = hook_identity_matches and readback_active is False
    hook_type = readback.get("type") if isinstance(readback.get("type"), str) else None
    target = config.get("url") if isinstance(config.get("url"), str) else None
    content_type = (
        config.get("content_type") if isinstance(config.get("content_type"), str) else None
    )
    configuration_matches = bool(
        hook_identity_matches
        and hook_type == "gitea"
        and target == expected_target
        and content_type == "json"
        and events_readback_valid
        and events is not None
        and len(events) == len(effective_hook_events(expected_event_selectors))
        and set(events) == effective_hook_events(expected_event_selectors)
    )
    return {
        "hook_id": hook_id,
        "api_hook_id": readback_id,
        "type": hook_type,
        "events": events,
        "events_readback_valid": events_readback_valid,
        "target": target,
        "content_type": content_type,
        "configuration_matches_expected": configuration_matches,
        "active_after_run": active_after_run,
        "deactivation_readback_confirmed": (
            deactivation_confirmed if hook_id is not None else None
        ),
        "secret_value_recorded": False,
    }


def private_config_with_test_allowlist(state: Path) -> None:
    config = state / "custom/conf/app.ini"
    flags = os.O_RDWR | os.O_APPEND | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    fd = os.open(config, flags)
    try:
        metadata = os.fstat(fd)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or metadata.st_nlink != 1
            or metadata.st_mode & 0o077
        ):
            raise RuntimeError("private Forgejo config has unsafe ownership or permissions")
        os.lseek(fd, 0, os.SEEK_SET)
        existing = bytearray()
        while chunk := os.read(fd, 65536):
            existing.extend(chunk)
            if len(existing) > 1024 * 1024:
                raise RuntimeError("private Forgejo config exceeds the local E2E size limit")
        if re.search(rb"(?m)^\s*\[webhook\]\s*$", existing, re.IGNORECASE):
            raise RuntimeError("private Forgejo config already contains a webhook section")
        addition = b"\n[webhook]\nALLOWED_HOST_LIST = 127.0.0.1\n"
        view = memoryview(addition)
        while view:
            written = os.write(fd, view)
            if written <= 0:
                raise OSError("short write while appending the test-only webhook allowlist")
            view = view[written:]
        os.fchmod(fd, 0o600)
        os.fsync(fd)
    finally:
        os.close(fd)


def run(args: argparse.Namespace) -> dict[str, Any]:
    ordinary_issue_comment = bool(getattr(args, "ordinary_issue_comment", False))
    edit_ordinary_issue_comment = bool(
        getattr(args, "edit_ordinary_issue_comment", False)
    )
    delete_ordinary_issue_comment = bool(
        getattr(args, "delete_ordinary_issue_comment", False)
    )
    if (edit_ordinary_issue_comment or delete_ordinary_issue_comment) and not ordinary_issue_comment:
        raise RuntimeError("ordinary comment edit or delete requires --ordinary-issue-comment")
    ordinary_issue_comment_actions = (
        ["created"]
        + (["edited"] if edit_ordinary_issue_comment else [])
        + (["deleted"] if delete_ordinary_issue_comment else [])
    )
    ordinary_issue_comment_event_count = ordinary_issue_comment_delivery_count(
        ordinary_issue_comment,
        edited=edit_ordinary_issue_comment,
        deleted=delete_ordinary_issue_comment,
    )
    synchronize_pull_request = bool(getattr(args, "synchronize_pull_request", False))
    reopen_pull_request = bool(getattr(args, "reopen_pull_request", False))
    reopen_issue = bool(getattr(args, "reopen_issue", False))
    edit_pull_request_title = bool(getattr(args, "edit_pull_request_title", False))
    change_pull_request_labels = bool(getattr(args, "change_pull_request_labels", False))
    change_pull_request_assignees = bool(
        getattr(args, "change_pull_request_assignees", False)
    )
    change_pull_request_milestones = bool(
        getattr(args, "change_pull_request_milestones", False)
    )
    change_issue_labels = bool(getattr(args, "change_issue_labels", False))
    change_issue_assignees = bool(getattr(args, "change_issue_assignees", False))
    change_issue_milestones = bool(getattr(args, "change_issue_milestones", False))
    issue_label_event_count = 2 * int(change_issue_labels)
    issue_assignee_event_count = 2 * int(change_issue_assignees)
    issue_milestone_event_count = 2 * int(change_issue_milestones)
    issue_mutation_event_count = (
        issue_label_event_count + issue_assignee_event_count + issue_milestone_event_count
    )
    issue_extra_event_count = issue_mutation_event_count + int(reopen_issue)
    remove_review_request = bool(getattr(args, "remove_review_request", False))
    request_review = bool(getattr(args, "request_review", False)) or remove_review_request
    request_changes_review = bool(getattr(args, "request_changes_review", False))
    approve_review = bool(getattr(args, "approve_review", False))
    fork_pull_request = bool(getattr(args, "fork_pull_request", False))
    if reopen_pull_request and bool(getattr(args, "merge_pull_request", False)):
        raise RuntimeError("cannot reopen a merged pull request")
    if request_changes_review and approve_review:
        raise RuntimeError("choose either --request-changes-review or --approve-review")
    pull_request_review_comment = bool(getattr(args, "pull_request_review_comment", False))
    if fork_pull_request and any(
        (
            synchronize_pull_request,
            reopen_pull_request,
            edit_pull_request_title,
            change_pull_request_labels,
            change_pull_request_assignees,
            change_pull_request_milestones,
            bool(getattr(args, "merge_pull_request", False)),
            request_changes_review,
            approve_review,
            request_review,
            pull_request_review_comment,
        )
    ):
        raise RuntimeError("--fork-pull-request currently runs without additional PR mutation scenarios")
    batch_pull_request_events = bool(getattr(args, "batch_pull_request_events", False))
    observe_batch_network = bool(getattr(args, "observe_batch_network", False))
    delete_pull_request_comment = bool(getattr(args, "delete_pull_request_comment", False))
    revoke_pull_request_comment_reader = bool(
        getattr(args, "revoke_pull_request_comment_reader_before_reconcile", False)
    )
    if observe_batch_network and not batch_pull_request_events:
        raise RuntimeError("batch network observation requires --batch-pull-request-events")
    if delete_pull_request_comment and batch_pull_request_events:
        raise RuntimeError("PR-comment deletion scenario requires individual event reconciliation")
    if revoke_pull_request_comment_reader and not delete_pull_request_comment:
        raise RuntimeError(
            "PR-comment reader revocation requires --delete-pull-request-comment"
        )
    expected_delivery_count = (
        7
        + int(synchronize_pull_request)
        + int(request_review)
        + int(remove_review_request)
        + int(request_changes_review)
        + int(approve_review)
        + int(pull_request_review_comment)
        + 2 * int(reopen_pull_request)
        + int(edit_pull_request_title)
        + int(delete_pull_request_comment)
        + ordinary_issue_comment_event_count
        + issue_extra_event_count
    )
    pre_trailing_delivery_count = expected_delivery_count - ordinary_issue_comment_event_count
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
    if shutil.which("rustc") is None or shutil.which("bwrap") is None:
        raise RuntimeError("rustc and Bubblewrap are required for the isolated local source build")
    strace_binary: Path | None = None
    strace_version: str | None = None
    if observe_batch_network:
        strace_path = shutil.which("strace")
        if strace_path is None:
            raise RuntimeError("strace is required for the requested batch network observation")
        strace_binary = Path(strace_path).resolve(strict=True)
        strace_version_result = subprocess.run(
            [str(strace_binary), "-V"],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            env={"PATH": "/usr/bin:/bin", "HOME": "/tmp"},
            timeout=5,
            check=False,
        )
        if strace_version_result.returncode != 0 or not strace_version_result.stdout.startswith("strace -- version "):
            raise RuntimeError("strace version could not be verified for the batch observer")
        strace_version = strace_version_result.stdout.splitlines()[0]
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
    private_config_with_test_allowlist(state)
    evidence_dir.mkdir(mode=0o700, parents=True)
    tokens: list[str] = []
    server_log = (state / "log/forgejo-server.log").open("wb")
    receiver_log = (state / "log/webhook-receiver.log").open("wb")
    server: subprocess.Popen | None = None
    receiver: subprocess.Popen | None = None
    receiver_port: int | None = None
    receiver_bin: Path | None = None
    reconcile_bin: Path | None = None
    build_provenance: dict[str, Any] | None = None
    server_stop: dict[str, Any] | None = None
    receiver_stop: dict[str, Any] | None = None
    receiver_stop_history: list[dict[str, Any]] = []
    owner_api: helpers.LocalForgejoApi | None = None
    hook_id: int | None = None
    hook_api_readback: dict[str, Any] | None = None
    hook_disabled = False
    cleanup_failure: str | None = None
    stage = "isolated_build"
    failure: str | None = None
    failure_message: str | None = None
    safe_result: dict[str, Any] | None = None
    journal_path = state / "data/webhook-inbox.jsonl"
    expected_hook_url: str | None = None
    issue_number: int | None = None
    initial_issue_event_content: dict[str, str] | None = None
    pull_request_number: int | None = None
    receiver_address: str | None = None
    reader_token: str | None = None
    contributor_token: str | None = None
    reader_scopes = "read:user,read:repository,read:issue"
    contributor_token_scopes = "read:user,write:issue,write:repository"
    reader_account: dict[str, Any] | None = None
    contributor_account: dict[str, Any] | None = None
    permissions: dict[str, Any] | None = None
    contributor_permissions: dict[str, Any] | None = None
    contributor_fork_repository: dict[str, Any] | None = None
    pull_request_base_repository_full_name: str | None = None
    pull_request_head_repository_full_name: str | None = None
    webhook_secret: str | None = None
    open_event: dict[str, Any] | None = None
    closed_event: dict[str, Any] | None = None
    issue_reopened_event: dict[str, Any] | None = None
    edited_events: list[dict[str, Any]] | None = None
    issue_label_update_event: dict[str, Any] | None = None
    issue_label_clear_event: dict[str, Any] | None = None
    issue_label_events: list[dict[str, Any]] = []
    issue_label_event_payloads: list[list[dict[str, Any]]] = []
    issue_label_api_readbacks: list[list[dict[str, Any]]] = []
    issue_label_event_statuses: list[int] = []
    issue_label_reconciliations: list[dict[str, Any]] = []
    issue_assignee_events: list[dict[str, Any]] = []
    issue_assignee_event_payloads: list[list[dict[str, Any]] | None] = []
    issue_assignee_api_readbacks: list[list[dict[str, Any]] | None] = []
    issue_assignee_event_statuses: list[int] = []
    issue_assignee_reconciliations: list[dict[str, Any]] = []
    issue_milestone_events: list[dict[str, Any]] = []
    issue_milestone_event_payloads: list[dict[str, Any] | None] = []
    issue_milestone_api_readbacks: list[dict[str, Any] | None] = []
    issue_milestone_event_statuses: list[int] = []
    issue_milestone_reconciliations: list[dict[str, Any]] = []
    issue_label_name: str | None = None
    issue_label_id: int | None = None
    issue_label_create_status: int | None = None
    issue_label_add_status: int | None = None
    issue_label_clear_status: int | None = None
    issue_assignee_add_status: int | None = None
    issue_assignee_clear_status: int | None = None
    issue_assignee_clear_api_verdict: str | None = None
    issue_milestone_title: str | None = None
    issue_milestone_id: int | None = None
    issue_milestone_create_status: int | None = None
    issue_milestone_assign_status: int | None = None
    issue_milestone_clear_status: int | None = None
    issue_milestone_clear_api_verdict: str | None = None
    ordinary_issue_comment_event: dict[str, Any] | None = None
    ordinary_issue_comment_events: list[dict[str, Any]] = []
    ordinary_issue_comment_id: int | None = None
    pull_request_open_event: dict[str, Any] | None = None
    pull_request_sync_event: dict[str, Any] | None = None
    pull_request_label_update_event: dict[str, Any] | None = None
    pull_request_label_clear_event: dict[str, Any] | None = None
    pull_request_label_events: list[dict[str, Any]] = []
    pull_request_label_event_payloads: list[list[dict[str, Any]]] = []
    pull_request_label_api_readbacks: list[list[dict[str, Any]]] = []
    pull_request_label_event_statuses: list[int] = []
    pull_request_label_reconciliations: list[dict[str, Any]] = []
    pull_request_assignee_events: list[dict[str, Any]] = []
    pull_request_assignee_event_payloads: list[list[dict[str, Any]] | None] = []
    pull_request_assignee_api_readbacks: list[list[dict[str, Any]] | None] = []
    pull_request_assignee_event_statuses: list[int] = []
    pull_request_assignee_reconciliations: list[dict[str, Any]] = []
    pull_request_assignee_clear_api_verdict: str | None = None
    pull_request_milestone_events: list[dict[str, Any]] = []
    pull_request_milestone_event_payloads: list[dict[str, Any] | None] = []
    pull_request_milestone_api_readbacks: list[dict[str, Any] | None] = []
    pull_request_milestone_event_statuses: list[int] = []
    pull_request_milestone_reconciliations: list[dict[str, Any]] = []
    pull_request_title_edit_event: dict[str, Any] | None = None
    pull_request_review_request_event: dict[str, Any] | None = None
    pull_request_review_request_removed_event: dict[str, Any] | None = None
    pull_request_comment_event: dict[str, Any] | None = None
    pull_request_comment_created_event: dict[str, Any] | None = None
    pull_request_comment_deleted_event: dict[str, Any] | None = None
    pull_request_review_comment_event: dict[str, Any] | None = None
    pull_request_rejected_review_event: dict[str, Any] | None = None
    pull_request_approved_review_event: dict[str, Any] | None = None
    pull_request_approved_review_response: dict[str, Any] | None = None
    pull_request_approved_review_http_status: int | None = None
    pull_request_pre_reopen_closed_event: dict[str, Any] | None = None
    pull_request_reopened_event: dict[str, Any] | None = None
    pull_request_closed_event: dict[str, Any] | None = None
    merge_response: dict[str, Any] | None = None
    pull_request_comment_id: int | None = None
    pull_request_comment_live_readback: dict[str, Any] | None = None
    pull_request_comment_delete_readback: dict[str, Any] | None = None
    pull_request_comment_delete_http_status: int | None = None
    pull_request_comment_parent_visibility: dict[str, Any] | None = None
    pull_request_comment_created_status: int | None = None
    pull_request_comment_created_reconciliation: dict[str, Any] | None = None
    open_reconciliation: dict[str, Any] | None = None
    closed_reconciliation: dict[str, Any] | None = None
    issue_reopened_status: int | None = None
    issue_reopened_reconciliation: dict[str, Any] | None = None
    edited_reconciliations: list[dict[str, Any]] | None = None
    ordinary_issue_comment_status: int | None = None
    ordinary_issue_comment_reconciliation: dict[str, Any] | None = None
    ordinary_issue_comment_delete_readback: dict[str, Any] | None = None
    ordinary_issue_comment_statuses: list[int] = []
    ordinary_issue_comment_reconciliations: list[dict[str, Any]] = []
    pull_request_open_reconciliation: dict[str, Any] | None = None
    pull_request_sync_status: int | None = None
    pull_request_sync_reconciliation: dict[str, Any] | None = None
    pull_request_label_name: str | None = None
    pull_request_label_id: int | None = None
    pull_request_label_create_status: int | None = None
    pull_request_label_add_status: int | None = None
    pull_request_label_clear_status: int | None = None
    pull_request_assignee_add_status: int | None = None
    pull_request_assignee_clear_status: int | None = None
    pull_request_milestone_title: str | None = None
    pull_request_milestone_id: int | None = None
    pull_request_milestone_create_status: int | None = None
    pull_request_milestone_assign_status: int | None = None
    pull_request_milestone_clear_status: int | None = None
    pull_request_milestone_clear_api_verdict: str | None = None
    pull_request_title_edit_status: int | None = None
    pull_request_title_edit_reconciliation: dict[str, Any] | None = None
    pull_request_title_edit_http_status: int | None = None
    pull_request_review_request_status: int | None = None
    pull_request_review_request_reconciliation: dict[str, Any] | None = None
    pull_request_review_request_removed_status: int | None = None
    pull_request_review_request_removed_reconciliation: dict[str, Any] | None = None
    requested_reviewers_readback: list[dict[str, Any]] | None = None
    requested_reviewers_after_removal_readback: list[dict[str, Any]] | None = None
    requested_reviewer_teams_readback: list[dict[str, Any]] | None = None
    review_request_http_status: int | None = None
    review_request_removal_http_status: int | None = None
    pull_request_comment_reconciliation: dict[str, Any] | None = None
    pull_request_review_comment_status: int | None = None
    pull_request_review_comment_reconciliation: dict[str, Any] | None = None
    pull_request_review_comment_response: dict[str, Any] | None = None
    pull_request_rejected_review_status: int | None = None
    pull_request_rejected_review_reconciliation: dict[str, Any] | None = None
    pull_request_approved_review_status: int | None = None
    pull_request_approved_review_reconciliation: dict[str, Any] | None = None
    pull_request_pre_reopen_closed_status: int | None = None
    pull_request_pre_reopen_closed_reconciliation: dict[str, Any] | None = None
    pull_request_reopened_status: int | None = None
    pull_request_reopened_reconciliation: dict[str, Any] | None = None
    pull_request_closed_reconciliation: dict[str, Any] | None = None
    pull_request_batch_reconciliation: dict[str, Any] | None = None
    batch_network_observation: dict[str, Any] | None = None
    pull_request_comment_status: int | None = None
    invalid_result: dict[str, Any] | None = None
    invalid_status: int | None = None
    duplicate_replay: dict[str, Any] | None = None
    hook_tasks: list[dict[str, Any]] = []

    try:
        receiver_bin, reconcile_bin, build_provenance = helpers.build_local_binaries(
            repo, state, evidence_dir
        )
        stage = "start_forgejo"
        server = subprocess.Popen(
            [
                str(forgejo),
                "--work-path",
                str(state),
                "--config",
                str(state / "custom/conf/app.ini"),
                "web",
            ],
            cwd=state,
            env=helpers.private_command_environment(state),
            stdout=server_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        helpers.forgejo_helpers.wait_for_health(server)
        helpers.forgejo_helpers.assert_forgejo_listener(server)

        stage = "provision_private_repository"
        helpers.admin_command(
            forgejo,
            state,
            [
                "admin",
                "user",
                "create",
                "--username",
                OWNER,
                "--random-password",
                "--email",
                f"{OWNER}@local.invalid",
                "--admin",
                "--must-change-password=false",
            ],
        )
        bootstrap_token = helpers.create_access_token(
            forgejo, state, OWNER, "emitted-webhook-bootstrap", "write:user,write:repository"
        )
        tokens.append(bootstrap_token)
        bootstrap_api = helpers.LocalForgejoApi(bootstrap_token)
        created_repo, _ = bootstrap_api.request(
            "POST",
            "/user/repos",
            {
                "name": REPOSITORY,
                "description": "Private local Forgejo-emitted webhook reconciliation E2E",
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
            forgejo,
            state,
            OWNER,
            "emitted-webhook-owner",
            "read:user,read:repository,read:issue,write:repository,write:issue",
        )
        tokens.append(owner_token)
        owner_api = helpers.LocalForgejoApi(owner_token)
        helpers.admin_command(
            forgejo,
            state,
            [
                "admin",
                "user",
                "create",
                "--username",
                READER,
                "--random-password",
                "--email",
                f"{READER}@local.invalid",
                "--must-change-password=false",
            ],
        )
        reader_token = helpers.create_access_token(
            forgejo, state, READER, "emitted-webhook-read-only", reader_scopes
        )
        tokens.append(reader_token)
        reader_api = helpers.LocalForgejoApi(reader_token)
        reader_account, _ = reader_api.request("GET", "/user")
        if reader_account.get("login") != READER or reader_account.get("is_admin") is not False:
            raise RuntimeError("Forgejo did not confirm the API reader is a non-admin")
        owner_api.request(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{READER}",
            {"permission": "read"},
        )
        repo_readback, _ = reader_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}")
        permissions = repo_readback.get("permissions")
        if (
            repo_readback.get("private") is not True
            or not isinstance(permissions, dict)
            or permissions.get("admin") is not False
            or permissions.get("push") is not False
            or permissions.get("pull") is not True
        ):
            raise RuntimeError("Forgejo did not confirm private read-only repository permissions")

        contributor_api = None
        if request_changes_review or approve_review or fork_pull_request:
            helpers.admin_command(
                forgejo,
                state,
                [
                    "admin",
                    "user",
                    "create",
                    "--username",
                    CONTRIBUTOR,
                    "--random-password",
                    "--email",
                    f"{CONTRIBUTOR}@local.invalid",
                    "--must-change-password=false",
                ],
            )
            contributor_token = helpers.create_access_token(
                forgejo,
                state,
                CONTRIBUTOR,
                "emitted-webhook-pr-author",
                contributor_token_scopes,
            )
            tokens.append(contributor_token)
            contributor_api = helpers.LocalForgejoApi(contributor_token)
            contributor_account, _ = contributor_api.request("GET", "/user")
            if (
                contributor_account.get("login") != CONTRIBUTOR
                or contributor_account.get("is_admin") is not False
            ):
                raise RuntimeError("Forgejo did not confirm the non-admin pull-request author")
            owner_api.request(
                "PUT",
                f"/repos/{OWNER}/{REPOSITORY}/collaborators/{CONTRIBUTOR}",
                {"permission": "read" if fork_pull_request else "write"},
            )
            contributor_repo, _ = contributor_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}"
            )
            contributor_permissions = contributor_repo.get("permissions")
            if (
                contributor_repo.get("private") is not True
                or not isinstance(contributor_permissions, dict)
                or contributor_permissions.get("admin") is not False
                or contributor_permissions.get("push") is not (not fork_pull_request)
                or contributor_permissions.get("pull") is not True
            ):
                raise RuntimeError("Forgejo did not confirm the expected PR-author repository permissions")
            if fork_pull_request:
                if contributor_api is None:
                    raise RuntimeError("the contributor API is unavailable for fork creation")
                created_fork, _ = contributor_api.request(
                    "POST", f"/repos/{OWNER}/{REPOSITORY}/forks", {}
                )
                if (
                    contributor_api.last_status != 202
                    or not isinstance(created_fork, dict)
                    or created_fork.get("full_name") != f"{CONTRIBUTOR}/{REPOSITORY}"
                ):
                    raise RuntimeError("Forgejo did not accept creation of the contributor fork")
                fork_path = f"/repos/{CONTRIBUTOR}/{REPOSITORY}"
                fork_deadline = time.monotonic() + 10
                while True:
                    fork_readback, _ = contributor_api.request(
                        "GET", fork_path, expected_http_error_statuses=(404,)
                    )
                    if contributor_api.last_status == 200:
                        break
                    if contributor_api.last_status != 404 or time.monotonic() >= fork_deadline:
                        raise RuntimeError("Forgejo did not make the accepted contributor fork readable")
                    time.sleep(0.1)
                parent = fork_readback.get("parent") if isinstance(fork_readback, dict) else None
                if (
                    not isinstance(fork_readback, dict)
                    or fork_readback.get("full_name") != f"{CONTRIBUTOR}/{REPOSITORY}"
                    or fork_readback.get("fork") is not True
                    or fork_readback.get("private") is not True
                    or not isinstance(parent, dict)
                    or parent.get("full_name") != f"{OWNER}/{REPOSITORY}"
                ):
                    raise RuntimeError("Forgejo fork readback did not bind the private parent repository")
                contributor_fork_repository = fork_readback

        stage = "start_receiver_and_create_hook"
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
                "events": ["issues", "pull_request"],
                "active": True,
            },
        )
        hook_id = hook_response.get("id")
        if not isinstance(hook_id, int) or hook_id < 1 or hook_response.get("active") is not True:
            raise RuntimeError("Forgejo did not create the expected active webhook")

        stage = "deliver_opened_issue_event"
        initial_issue_title = "Forgejo-emitted reconciliation"
        initial_issue_body = "The receiver must journal this real event."
        created_issue, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/issues",
            {"title": initial_issue_title, "body": initial_issue_body},
        )
        issue_number = created_issue.get("number")
        if not isinstance(issue_number, int) or isinstance(issue_number, bool) or issue_number < 1:
            raise RuntimeError("Forgejo did not return the expected created issue number")
        open_records = wait_for_journal_count(journal_path, 1)
        first_open_candidate = open_records[0]
        if not isinstance(first_open_candidate.get("payload"), dict):
            raise RuntimeError("Forgejo delivery journal did not contain a payload")
        initial_payload_issue = first_open_candidate["payload"].get("payload", {}).get("issue")
        if (
            not isinstance(initial_payload_issue, dict)
            or initial_payload_issue.get("title") != initial_issue_title
            or initial_payload_issue.get("body") != initial_issue_body
        ):
            raise RuntimeError("Forgejo's opened webhook did not preserve the initial issue content")
        initial_issue_event_content = {
            "title": initial_issue_title,
            "body": initial_issue_body,
        }
        open_event = parse_live_delivery_record(
            first_open_candidate,
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
            expected_action="opened",
        )

        if change_issue_labels:
            stage = "deliver_issue_label_updated_event"
            issue_label_name = "agent-forge-e2e-live-issue-label"
            created_label, _ = owner_api.request(
                "POST",
                f"/repos/{OWNER}/{REPOSITORY}/labels",
                {
                    "name": issue_label_name,
                    "color": "536dfe",
                    "description": "Disposable local issue label webhook fixture",
                },
            )
            issue_label_create_status = owner_api.last_status
            issue_label_id = created_label.get("id") if isinstance(created_label, dict) else None
            if (
                issue_label_create_status != 201
                or type(issue_label_id) is not int
                or issue_label_id < 1
                or created_label.get("name") != issue_label_name
            ):
                raise RuntimeError("Forgejo did not confirm creation of the disposable issue label")

            records_before_issue_label_update = read_journal_records(journal_path)
            owner_api.request(
                "POST",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}/labels",
                {"labels": [issue_label_name]},
            )
            issue_label_add_status = owner_api.last_status
            issue_after_label_add, _ = owner_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
            )
            issue_labels_after_add = normalize_live_issue_labels(
                issue_after_label_add.get("labels")
                if isinstance(issue_after_label_add, dict)
                else None
            )
            expected_issue_label = {"id": issue_label_id, "name": issue_label_name}
            if (
                issue_label_add_status != 200
                or expected_issue_label not in issue_labels_after_add
                or issue_after_label_add.get("state") != "open"
            ):
                raise RuntimeError("Forgejo issue API did not confirm the added label")

            records_after_issue_label_update = wait_for_journal_count(
                journal_path, len(records_before_issue_label_update) + 1
            )
            if len(records_after_issue_label_update) != len(records_before_issue_label_update) + 1:
                raise RuntimeError("Forgejo did not emit exactly one issues/label_updated event")
            issue_label_update_event = parse_live_delivery_record(
                records_after_issue_label_update[-1],
                owner=OWNER,
                repository=REPOSITORY,
                issue_number=issue_number,
                expected_action="label_updated",
            )
            if (
                issue_label_update_event["payload_labels"] != issue_labels_after_add
                or expected_issue_label not in issue_label_update_event["payload_labels"]
                or issue_label_update_event["sequence"] <= open_event["sequence"]
                or issue_label_update_event["delivery_id"] == open_event["delivery_id"]
            ):
                raise RuntimeError("Forgejo issue label_updated event differs from API readback")
            issue_label_events.append(issue_label_update_event)
            issue_label_event_payloads.append(issue_label_update_event["payload_labels"])
            issue_label_api_readbacks.append(issue_labels_after_add)

            stage = "deliver_issue_label_cleared_event"
            records_before_issue_label_clear = read_journal_records(journal_path)
            owner_api.request(
                "DELETE",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}/labels",
                {},
            )
            issue_label_clear_status = owner_api.last_status
            issue_after_label_clear, _ = owner_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
            )
            issue_labels_after_clear = normalize_live_issue_labels(
                issue_after_label_clear.get("labels")
                if isinstance(issue_after_label_clear, dict)
                else None
            )
            if issue_label_clear_status != 204 or issue_labels_after_clear:
                raise RuntimeError("Forgejo issue API did not confirm clearing all labels")

            records_after_issue_label_clear = wait_for_journal_count(
                journal_path, len(records_before_issue_label_clear) + 1
            )
            if len(records_after_issue_label_clear) != len(records_before_issue_label_clear) + 1:
                raise RuntimeError("Forgejo did not emit exactly one issues/label_cleared event")
            issue_label_clear_event = parse_live_delivery_record(
                records_after_issue_label_clear[-1],
                owner=OWNER,
                repository=REPOSITORY,
                issue_number=issue_number,
                expected_action="label_cleared",
            )
            validate_live_issue_label_clear_snapshot(
                clear_payload_labels=issue_label_clear_event["payload_labels"],
                update_payload_labels=issue_label_update_event["payload_labels"],
                api_labels_after_clear=issue_labels_after_clear,
                expected_label_id=issue_label_id,
                expected_label_name=issue_label_name,
            )
            if (
                issue_label_clear_event["sequence"] <= issue_label_update_event["sequence"]
                or issue_label_clear_event["delivery_id"] == issue_label_update_event["delivery_id"]
            ):
                raise RuntimeError("Forgejo issue label_cleared event is duplicated or out of order")
            issue_label_events.append(issue_label_clear_event)
            issue_label_event_payloads.append(issue_label_clear_event["payload_labels"])
            issue_label_api_readbacks.append(issue_labels_after_clear)

        if change_issue_assignees:
            stage = "deliver_issue_assigned_event"
            assignee_id = reader_account.get("id") if isinstance(reader_account, dict) else None
            if type(assignee_id) is not int or assignee_id < 1:
                raise RuntimeError("Forgejo did not return a valid read-only assignee identity")
            expected_assignee = {"id": assignee_id, "login": READER}
            records_before_issue_assignee_update = read_journal_records(journal_path)
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}",
                {"assignees": [READER]},
            )
            issue_assignee_add_status = owner_api.last_status
            issue_after_assignee_add, _ = owner_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
            )
            issue_assignees_after_add = normalize_current_issue_api_assignees(
                issue_after_assignee_add
            )
            if (
                issue_assignee_add_status != 201
                or issue_assignees_after_add != [expected_assignee]
                or issue_after_assignee_add.get("state") != "open"
            ):
                raise RuntimeError("Forgejo issue API did not confirm the disposable assignment")

            records_after_issue_assignee_update = wait_for_journal_count(
                journal_path, len(records_before_issue_assignee_update) + 1
            )
            if len(records_after_issue_assignee_update) != len(records_before_issue_assignee_update) + 1:
                raise RuntimeError("Forgejo did not emit exactly one issues/assigned event")
            issue_assigned_event = parse_live_delivery_record(
                records_after_issue_assignee_update[-1],
                owner=OWNER,
                repository=REPOSITORY,
                issue_number=issue_number,
                expected_action="assigned",
            )
            previous_issue_event = issue_label_events[-1] if issue_label_events else open_event
            if (
                issue_assigned_event["payload_assignees"] != issue_assignees_after_add
                or issue_assigned_event["sequence"] <= previous_issue_event["sequence"]
                or issue_assigned_event["delivery_id"]
                in {open_event["delivery_id"], *(event["delivery_id"] for event in issue_label_events)}
            ):
                raise RuntimeError("Forgejo issues/assigned event differs from the current API readback")
            issue_assignee_events.append(issue_assigned_event)
            issue_assignee_event_payloads.append(issue_assigned_event["payload_assignees"])
            issue_assignee_api_readbacks.append(issue_assignees_after_add)

            stage = "deliver_issue_unassigned_event"
            records_before_issue_assignee_clear = read_journal_records(journal_path)
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}",
                {"assignees": []},
            )
            issue_assignee_clear_status = owner_api.last_status
            issue_after_assignee_clear, _ = owner_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
            )
            issue_assignees_after_clear = normalize_current_issue_api_assignees(
                issue_after_assignee_clear
            )
            if issue_assignee_clear_status != 201:
                raise RuntimeError(
                    "Forgejo issue API did not accept clearing assignees "
                    f"(patch_status={issue_assignee_clear_status})"
                )

            records_after_issue_assignee_clear = wait_for_journal_count(
                journal_path, len(records_before_issue_assignee_clear) + 1
            )
            if len(records_after_issue_assignee_clear) != len(records_before_issue_assignee_clear) + 1:
                raise RuntimeError("Forgejo did not emit exactly one issues/unassigned event")
            issue_unassigned_event = parse_live_delivery_record(
                records_after_issue_assignee_clear[-1],
                owner=OWNER,
                repository=REPOSITORY,
                issue_number=issue_number,
                expected_action="unassigned",
            )
            try:
                issue_assignee_clear_api_verdict = validate_live_issue_assignee_clear_snapshot(
                    clear_payload_assignees=issue_unassigned_event["payload_assignees"],
                    update_payload_assignees=issue_assigned_event["payload_assignees"],
                    api_assignees_after_clear=issue_assignees_after_clear,
                    expected_assignee=expected_assignee,
                )
            except ValueError as error:
                raise RuntimeError(str(error)) from None
            if (
                issue_unassigned_event["sequence"] <= issue_assigned_event["sequence"]
                or issue_unassigned_event["delivery_id"] == issue_assigned_event["delivery_id"]
            ):
                raise RuntimeError("Forgejo issues/unassigned event is duplicated or out of order")
            issue_assignee_events.append(issue_unassigned_event)
            issue_assignee_event_payloads.append(issue_unassigned_event["payload_assignees"])
            issue_assignee_api_readbacks.append(issue_assignees_after_clear)

        if change_issue_milestones:
            stage = "deliver_issue_milestoned_event"
            issue_milestone_title = "agent-forge-e2e-live-issue-milestone"
            created_milestone, _ = owner_api.request(
                "POST",
                f"/repos/{OWNER}/{REPOSITORY}/milestones",
                {
                    "title": issue_milestone_title,
                    "description": "Disposable local issue milestone webhook fixture",
                },
            )
            issue_milestone_create_status = owner_api.last_status
            issue_milestone_id = (
                created_milestone.get("id") if isinstance(created_milestone, dict) else None
            )
            if (
                issue_milestone_create_status != 201
                or type(issue_milestone_id) is not int
                or issue_milestone_id < 1
                or created_milestone.get("title") != issue_milestone_title
                or created_milestone.get("state") != "open"
            ):
                raise RuntimeError("Forgejo did not confirm creation of the disposable milestone")

            records_before_issue_milestone_assign = read_journal_records(journal_path)
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}",
                {"milestone": issue_milestone_id},
            )
            issue_milestone_assign_status = owner_api.last_status
            issue_after_milestone_assign, _ = owner_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
            )
            if (
                not isinstance(issue_after_milestone_assign, dict)
                or "milestone" not in issue_after_milestone_assign
            ):
                raise RuntimeError("Forgejo issue API omitted its milestone field after assignment")
            issue_milestone_after_assign = normalize_live_issue_milestone(
                issue_after_milestone_assign["milestone"]
            )
            expected_issue_milestone = {
                "id": issue_milestone_id,
                "title": issue_milestone_title,
                "state": "open",
            }
            if (
                issue_milestone_assign_status != 201
                or issue_milestone_after_assign != expected_issue_milestone
                or issue_after_milestone_assign.get("state") != "open"
            ):
                raise RuntimeError("Forgejo issue API did not confirm the assigned milestone")

            records_after_issue_milestone_assign = wait_for_journal_count(
                journal_path, len(records_before_issue_milestone_assign) + 1
            )
            if len(records_after_issue_milestone_assign) != len(records_before_issue_milestone_assign) + 1:
                raise RuntimeError("Forgejo did not emit exactly one issues/milestoned event")
            issue_milestoned_event = parse_live_delivery_record(
                records_after_issue_milestone_assign[-1],
                owner=OWNER,
                repository=REPOSITORY,
                issue_number=issue_number,
                expected_action="milestoned",
            )
            previous_issue_event = (
                issue_assignee_events[-1]
                if issue_assignee_events
                else issue_label_events[-1]
                if issue_label_events
                else open_event
            )
            prior_issue_delivery_ids = {
                open_event["delivery_id"],
                *(event["delivery_id"] for event in issue_label_events),
                *(event["delivery_id"] for event in issue_assignee_events),
            }
            if (
                issue_milestoned_event["payload_milestone"] != issue_milestone_after_assign
                or issue_milestoned_event["sequence"] <= previous_issue_event["sequence"]
                or issue_milestoned_event["delivery_id"] in prior_issue_delivery_ids
            ):
                raise RuntimeError("Forgejo issues/milestoned event differs from current API readback")
            issue_milestone_events.append(issue_milestoned_event)
            issue_milestone_event_payloads.append(issue_milestoned_event["payload_milestone"])
            issue_milestone_api_readbacks.append(issue_milestone_after_assign)

            stage = "deliver_issue_demilestoned_event"
            records_before_issue_milestone_clear = read_journal_records(journal_path)
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}",
                {"milestone": 0},
            )
            issue_milestone_clear_status = owner_api.last_status
            issue_after_milestone_clear, _ = owner_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
            )
            if (
                not isinstance(issue_after_milestone_clear, dict)
                or "milestone" not in issue_after_milestone_clear
            ):
                raise RuntimeError("Forgejo issue API omitted its milestone field after clearing")
            issue_milestone_after_clear = normalize_live_issue_milestone(
                issue_after_milestone_clear["milestone"]
            )
            if issue_milestone_clear_status != 201 or issue_milestone_after_clear is not None:
                raise RuntimeError("Forgejo issue API did not confirm clearing the milestone")

            records_after_issue_milestone_clear = wait_for_journal_count(
                journal_path, len(records_before_issue_milestone_clear) + 1
            )
            if len(records_after_issue_milestone_clear) != len(records_before_issue_milestone_clear) + 1:
                raise RuntimeError("Forgejo did not emit exactly one issues/demilestoned event")
            issue_demilestoned_event = parse_live_delivery_record(
                records_after_issue_milestone_clear[-1],
                owner=OWNER,
                repository=REPOSITORY,
                issue_number=issue_number,
                expected_action="demilestoned",
            )
            try:
                issue_milestone_clear_api_verdict = validate_live_issue_milestone_clear_snapshot(
                    clear_payload_milestone=issue_demilestoned_event["payload_milestone"],
                    update_payload_milestone=issue_milestoned_event["payload_milestone"],
                    api_milestone_after_clear=issue_milestone_after_clear,
                    expected_milestone_id=issue_milestone_id,
                    expected_milestone_title=issue_milestone_title,
                )
            except ValueError as error:
                raise RuntimeError(str(error)) from None
            if (
                issue_demilestoned_event["sequence"] <= issue_milestoned_event["sequence"]
                or issue_demilestoned_event["delivery_id"]
                == issue_milestoned_event["delivery_id"]
            ):
                raise RuntimeError("Forgejo issues/demilestoned event is duplicated or out of order")
            issue_milestone_events.append(issue_demilestoned_event)
            issue_milestone_event_payloads.append(issue_demilestoned_event["payload_milestone"])
            issue_milestone_api_readbacks.append(issue_milestone_after_clear)

        stage = "deliver_closed_issue_event"
        owner_api.request(
            "PATCH",
            f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}",
            {"state": "closed"},
        )
        closed_issue_record_count = 2 + issue_mutation_event_count
        all_records = wait_for_journal_count(journal_path, closed_issue_record_count)
        if len(all_records) != closed_issue_record_count:
            raise RuntimeError("Forgejo delivered an unexpected number of issue events")
        closed_event = parse_live_delivery_record(
            all_records[1 + issue_mutation_event_count],
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
            expected_action="closed",
        )
        if open_event["delivery_id"] == closed_event["delivery_id"]:
            raise RuntimeError("Forgejo reused a delivery ID across distinct issue events")
        if open_event["sequence"] >= closed_event["sequence"]:
            raise RuntimeError("receiver journal sequence did not preserve issue transition order")

        stage = "deliver_edited_closed_issue_event"
        edited_title = "Forgejo-emitted reconciliation (edited)"
        edited_body = "Editing a closed issue emits a notification; reconcile from Forgejo's current API state."
        owner_api.request(
            "PATCH",
            f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}",
            {"title": edited_title, "body": edited_body},
        )
        issue_event_count_after_edits = 4 + issue_mutation_event_count
        records_after_issue_edit = wait_for_journal_count(journal_path, issue_event_count_after_edits)
        if len(records_after_issue_edit) != issue_event_count_after_edits:
            raise RuntimeError("Forgejo delivered an unexpected number of issue events before PR creation")
        edited_events = [
            parse_live_delivery_record(
                record,
                owner=OWNER,
                repository=REPOSITORY,
                issue_number=issue_number,
                expected_action="edited",
            )
            for record in records_after_issue_edit[2 + issue_mutation_event_count :]
        ]
        if (
            len({event["delivery_id"] for event in edited_events}) != 2
            or any(event["payload_state"] != "closed" for event in edited_events)
            or edited_events[0]["sequence"] <= closed_event["sequence"]
            or edited_events[0]["sequence"] >= edited_events[1]["sequence"]
        ):
            raise RuntimeError("Forgejo edited-issue events did not preserve closed state and delivery order")

        issue_reopen_http_status: int | None = None
        issue_reopened_api_readback: dict[str, Any] | None = None
        if reopen_issue:
            stage = "deliver_issue_reopened_event"
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}",
                {"state": "open"},
            )
            issue_reopen_http_status = owner_api.last_status
            issue_reopened_api_readback, _ = owner_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
            )
            if (
                issue_reopen_http_status != 201
                or not isinstance(issue_reopened_api_readback, dict)
                or issue_reopened_api_readback.get("state") != "open"
                or issue_reopened_api_readback.get("number") != issue_number
            ):
                raise RuntimeError("Forgejo API did not confirm reopening the issue")

            issue_reopened_record_count = issue_event_count_after_edits + 1
            records_after_issue_reopen = wait_for_journal_count(
                journal_path, issue_reopened_record_count
            )
            if len(records_after_issue_reopen) != issue_reopened_record_count:
                raise RuntimeError("Forgejo did not emit exactly one issue reopened event")
            issue_reopened_event = parse_live_delivery_record(
                records_after_issue_reopen[-1],
                owner=OWNER,
                repository=REPOSITORY,
                issue_number=issue_number,
                expected_action="reopened",
            )
            if (
                issue_reopened_event["payload_state"] != "open"
                or issue_reopened_event["delivery_id"]
                in {open_event["delivery_id"], closed_event["delivery_id"]}
                or issue_reopened_event["sequence"] <= edited_events[-1]["sequence"]
            ):
                raise RuntimeError("Forgejo issue reopen event is duplicated, stale, or out of order")

        stage = "create_pull_request_candidate"
        branch_name = "agent/webhook-e2e"
        main_branch, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main"
        )
        base_sha_at_creation = main_branch.get("commit", {}).get("id")
        if not isinstance(base_sha_at_creation, str) or not COMMIT_SHA_RE.fullmatch(base_sha_at_creation):
            raise RuntimeError("Forgejo main branch readback did not contain a valid commit SHA")
        candidate_api = contributor_api if fork_pull_request else owner_api
        candidate_repository_path = (
            f"/repos/{CONTRIBUTOR}/{REPOSITORY}"
            if fork_pull_request
            else f"/repos/{OWNER}/{REPOSITORY}"
        )
        if candidate_api is None:
            raise RuntimeError("the scoped contributor API is unavailable for the fork candidate")
        if fork_pull_request:
            fork_main, _ = candidate_api.request(
                "GET", f"{candidate_repository_path}/branches/main"
            )
            if fork_main.get("commit", {}).get("id") != base_sha_at_creation:
                raise RuntimeError("the fork default branch does not match the target base snapshot")
        candidate_api.request(
            "POST",
            f"{candidate_repository_path}/branches",
            {"new_branch_name": branch_name, "old_branch_name": "main"},
        )
        candidate_content = (
            "# Forgejo webhook E2E\n\n"
            "This file is a real candidate change used to exercise pull-request events.\n"
        ).encode("utf-8")
        candidate_api.request(
            "POST",
            f"{candidate_repository_path}/contents/webhook-e2e.txt",
            {
                "message": "Add webhook E2E candidate file",
                "content": base64.b64encode(candidate_content).decode("ascii"),
                "branch": branch_name,
            },
        )
        candidate_file, _ = candidate_api.request(
            "GET",
            f"{candidate_repository_path}/contents/webhook-e2e.txt?ref={branch_name}",
        )
        if (
            not isinstance(candidate_file, dict)
            or base64.b64decode(candidate_file.get("content", ""), validate=True)
            != candidate_content
        ):
            raise RuntimeError("Forgejo source readback differs from the PR candidate change")
        pull_request_author_api = contributor_api or owner_api
        created_pull_request, _ = pull_request_author_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/pulls",
            {
                "title": "Exercise emitted pull-request webhooks",
                "body": "Verify that the live receiver journals and reconciles PR transitions.",
                "head": f"{CONTRIBUTOR}:{branch_name}" if fork_pull_request else branch_name,
                "base": "main",
            },
        )
        pull_request_number = created_pull_request.get("number")
        if (
            not isinstance(pull_request_number, int)
            or isinstance(pull_request_number, bool)
            or pull_request_number < 1
        ):
            raise RuntimeError("Forgejo did not return the expected pull-request number")
        pull_request_path = f"/repos/{OWNER}/{REPOSITORY}/pulls/{pull_request_number}"
        opened_pull_request, _ = owner_api.request("GET", pull_request_path)
        requested_reviewer_teams_readback = normalize_requested_reviewer_teams_readback(
            opened_pull_request
        )
        opened_head_sha = opened_pull_request.get("head", {}).get("sha")
        opened_base_sha = opened_pull_request.get("base", {}).get("sha")
        opened_base_repository = opened_pull_request.get("base", {}).get("repo")
        opened_head_repository = opened_pull_request.get("head", {}).get("repo")
        pull_request_base_repository_full_name = (
            opened_base_repository.get("full_name")
            if isinstance(opened_base_repository, dict)
            else None
        )
        pull_request_head_repository_full_name = (
            opened_head_repository.get("full_name")
            if isinstance(opened_head_repository, dict)
            else None
        )
        if (
            opened_pull_request.get("state") != "open"
            or opened_pull_request.get("merged") is not False
            or not isinstance(opened_head_sha, str)
            or not COMMIT_SHA_RE.fullmatch(opened_head_sha)
            or opened_head_sha == base_sha_at_creation
            or opened_base_sha != base_sha_at_creation
        ):
            raise RuntimeError("Forgejo PR API readback did not confirm the candidate and base commits")
        expected_base_repository_full_name = f"{OWNER}/{REPOSITORY}"
        expected_head_repository_full_name = (
            f"{CONTRIBUTOR}/{REPOSITORY}"
            if fork_pull_request
            else expected_base_repository_full_name
        )
        if (
            pull_request_base_repository_full_name != expected_base_repository_full_name
            or pull_request_head_repository_full_name != expected_head_repository_full_name
        ):
            raise RuntimeError("Forgejo PR API readback did not preserve the expected base/head repositories")

        stage = "deliver_opened_pull_request_event"
        open_pr_record_count = 5 + issue_extra_event_count
        records_after_open_pr = wait_for_journal_count(journal_path, open_pr_record_count)
        if len(records_after_open_pr) != open_pr_record_count:
            raise RuntimeError("Forgejo delivered an unexpected event before PR close")
        pull_request_open_event = parse_live_pull_request_delivery_record(
            records_after_open_pr[4 + issue_extra_event_count],
            owner=OWNER,
            repository=REPOSITORY,
            pull_request_number=pull_request_number,
            expected_action="opened",
            expected_base_repository_full_name=(
                expected_base_repository_full_name if fork_pull_request else None
            ),
            expected_head_repository_full_name=(
                expected_head_repository_full_name if fork_pull_request else None
            ),
        )
        if (
            pull_request_open_event["payload_state"] != "open"
            or pull_request_open_event["payload_merged"] is not False
            or pull_request_open_event["head_sha"] != opened_head_sha
            or pull_request_open_event["base_sha"] != opened_base_sha
        ):
            raise RuntimeError("emitted opened-PR payload differs from Forgejo's PR readback")

        synchronized_content = candidate_content
        candidate_head_sha = opened_head_sha
        if synchronize_pull_request:
            stage = "synchronize_pull_request_candidate"
            if not isinstance(candidate_file.get("sha"), str) or not COMMIT_SHA_RE.fullmatch(
                candidate_file["sha"]
            ):
                raise RuntimeError("Forgejo candidate file readback omitted its object SHA")
            synchronized_content = candidate_content + b"\nA second commit exercises Forgejo's native PR sync event.\n"
            owner_api.request(
                "PUT",
                f"/repos/{OWNER}/{REPOSITORY}/contents/webhook-e2e.txt",
                {
                    "message": "Update candidate to exercise PR synchronization",
                    "content": base64.b64encode(synchronized_content).decode("ascii"),
                    "branch": branch_name,
                    "sha": candidate_file["sha"],
                },
            )
            synchronized_file, _ = owner_api.request(
                "GET",
                f"/repos/{OWNER}/{REPOSITORY}/contents/webhook-e2e.txt?ref={branch_name}",
            )
            synchronized_pull_request, _ = owner_api.request("GET", pull_request_path)
            candidate_head_sha = synchronized_pull_request.get("head", {}).get("sha")
            if (
                not isinstance(synchronized_file, dict)
                or base64.b64decode(synchronized_file.get("content", ""), validate=True)
                != synchronized_content
                or not isinstance(candidate_head_sha, str)
                or not COMMIT_SHA_RE.fullmatch(candidate_head_sha)
                or candidate_head_sha == opened_head_sha
                or synchronized_pull_request.get("state") != "open"
                or synchronized_pull_request.get("merged") is not False
                or synchronized_pull_request.get("base", {}).get("sha") != opened_base_sha
            ):
                raise RuntimeError("Forgejo API readback did not confirm an advanced open PR head")
            sync_record_count = 6 + issue_extra_event_count
            records_after_sync = wait_for_journal_count(journal_path, sync_record_count)
            if len(records_after_sync) != sync_record_count:
                raise RuntimeError("Forgejo did not emit exactly one PR synchronization event")
            pull_request_sync_event = parse_live_pull_request_delivery_record(
                records_after_sync[5 + issue_extra_event_count],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="synchronized",
                expected_event="pull_request",
                expected_merged=False,
            )
            if (
                pull_request_sync_event["sequence"] <= pull_request_open_event["sequence"]
                or pull_request_sync_event["delivery_id"] == pull_request_open_event["delivery_id"]
                or pull_request_sync_event["payload_state"] != "open"
                or pull_request_sync_event["payload_merged"] is not False
                or pull_request_sync_event["head_sha"] != candidate_head_sha
                or pull_request_sync_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError("Forgejo synchronization event differs from the advanced PR readback")

        stage = "deliver_pull_request_conversation_comment"
        created_comment, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/issues/{pull_request_number}/comments",
            {"body": "Local Forgejo webhook adapter reconciliation fixture comment."},
        )
        pull_request_comment_id = created_comment.get("id")
        if (
            not isinstance(pull_request_comment_id, int)
            or isinstance(pull_request_comment_id, bool)
            or pull_request_comment_id < 1
        ):
            raise RuntimeError("Forgejo did not return the expected pull-request comment ID")
        pull_request_comment_live_readback, _ = owner_api.request(
            "GET",
            f"/repos/{OWNER}/{REPOSITORY}/issues/comments/{pull_request_comment_id}",
        )
        created_comment_record_count = (
            6 + issue_extra_event_count + int(synchronize_pull_request)
        )
        records_after_pr_comment = wait_for_journal_count(
            journal_path, created_comment_record_count
        )
        if len(records_after_pr_comment) != created_comment_record_count:
            raise RuntimeError("Forgejo did not emit exactly one pull-request conversation comment")
        pull_request_comment_created_event = parse_live_pull_request_delivery_record(
            records_after_pr_comment[created_comment_record_count - 1],
            owner=OWNER,
            repository=REPOSITORY,
            pull_request_number=pull_request_number,
            expected_action="created",
            expected_event="issue_comment",
            expected_comment_id=pull_request_comment_id,
        )
        if (
            pull_request_comment_created_event["sequence"]
            <= pull_request_open_event["sequence"]
            or (
                pull_request_sync_event is not None
                and pull_request_comment_created_event["sequence"]
                <= pull_request_sync_event["sequence"]
            )
            or pull_request_comment_created_event["payload_state"] != "open"
            or pull_request_comment_created_event["payload_merged"] is not False
            or pull_request_comment_created_event["head_sha"] != candidate_head_sha
            or pull_request_comment_created_event["base_sha"] != opened_base_sha
        ):
            raise RuntimeError("Forgejo conversation comment event did not bind to the open PR candidate")
        comment_record_count = created_comment_record_count
        pull_request_comment_event = pull_request_comment_created_event
        if delete_pull_request_comment:
            stage = "delete_pull_request_conversation_comment"
            owner_api.request(
                "DELETE",
                f"/repos/{OWNER}/{REPOSITORY}/issues/comments/{pull_request_comment_id}",
            )
            if owner_api.last_status != 204:
                raise RuntimeError("Forgejo did not confirm deleting the PR conversation comment")
            owner_api.request(
                "GET",
                f"/repos/{OWNER}/{REPOSITORY}/issues/comments/{pull_request_comment_id}",
                expected_http_error_statuses=(404,),
            )
            pull_request_comment_delete_http_status = owner_api.last_status
            pull_request_comment_delete_readback = validate_issue_comment_delete_readback(
                pull_request_comment_delete_http_status, pull_request_comment_id
            )
            comment_record_count += 1
            records_after_pr_comment_delete = wait_for_journal_count(
                journal_path, comment_record_count
            )
            if len(records_after_pr_comment_delete) != comment_record_count:
                raise RuntimeError("Forgejo did not emit exactly one deleted PR conversation comment")
            pull_request_comment_deleted_event = parse_live_pull_request_delivery_record(
                records_after_pr_comment_delete[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="deleted",
                expected_event="issue_comment",
                expected_comment_id=pull_request_comment_id,
            )
            if (
                pull_request_comment_deleted_event["sequence"]
                <= pull_request_comment_created_event["sequence"]
                or pull_request_comment_deleted_event["delivery_id"]
                == pull_request_comment_created_event["delivery_id"]
                or pull_request_comment_deleted_event["payload_state"] != "open"
                or pull_request_comment_deleted_event["payload_merged"] is not False
                or pull_request_comment_deleted_event["head_sha"] != candidate_head_sha
                or pull_request_comment_deleted_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError("Forgejo deleted-comment event is duplicated or mismatched")
            pull_request_comment_event = pull_request_comment_deleted_event

        if pull_request_review_comment:
            stage = "deliver_pull_request_review_comment_event"
            pull_request_review_comment_response, _ = owner_api.request(
                "POST",
                f"{pull_request_path}/reviews",
                {
                    "event": "COMMENT",
                    "body": "Local Forgejo inline review comment fixture.",
                    "commit_id": candidate_head_sha,
                    "comments": [
                        {
                            "path": "webhook-e2e.txt",
                            "body": "Local fixture review note on the candidate line.",
                            "new_position": 1,
                        }
                    ],
                },
            )
            review_comment_record_count = comment_record_count + 1
            records_after_review_comment = wait_for_journal_count(
                journal_path, review_comment_record_count
            )
            if len(records_after_review_comment) != review_comment_record_count:
                raise RuntimeError("Forgejo did not emit exactly one inline review-comment event")
            pull_request_review_comment_event = parse_live_pull_request_delivery_record(
                records_after_review_comment[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="reviewed",
                expected_event="pull_request_comment",
                expected_merged=False,
            )
            if (
                pull_request_review_comment_response.get("state") != "COMMENT"
                or pull_request_review_comment_response.get("commit_id") != candidate_head_sha
                or pull_request_review_comment_response.get("user", {}).get("login") != OWNER
                or pull_request_review_comment_event["sequence"]
                <= pull_request_comment_event["sequence"]
                or pull_request_review_comment_event["delivery_id"]
                == pull_request_comment_event["delivery_id"]
                or pull_request_review_comment_event["payload_state"] != "open"
                or pull_request_review_comment_event["payload_merged"] is not False
                or pull_request_review_comment_event["head_sha"] != candidate_head_sha
                or pull_request_review_comment_event["base_sha"] != opened_base_sha
                or pull_request_review_comment_event["review_type"]
                != "pull_request_review_comment"
            ):
                raise RuntimeError("Forgejo inline review-comment event differs from its API readback")

        if request_changes_review:
            stage = "deliver_pull_request_rejected_review_event"
            review_response, _ = owner_api.request(
                "POST",
                f"{pull_request_path}/reviews",
                {
                    "event": "REQUEST_CHANGES",
                    "body": "The isolated reviewer requests one concrete correction.",
                    "commit_id": candidate_head_sha,
                },
            )
            review_record_count = comment_record_count + 1 + int(pull_request_review_comment)
            records_after_rejected_review = wait_for_journal_count(
                journal_path, review_record_count
            )
            if len(records_after_rejected_review) != review_record_count:
                raise RuntimeError("Forgejo did not emit exactly one rejected-review event")
            pull_request_rejected_review_event = parse_live_pull_request_delivery_record(
                records_after_rejected_review[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="reviewed",
                expected_event="pull_request_rejected",
                expected_merged=False,
            )
            if (
                review_response.get("state") != "REQUEST_CHANGES"
                or review_response.get("commit_id") != candidate_head_sha
                or review_response.get("user", {}).get("login") != OWNER
                or pull_request_rejected_review_event["sequence"]
                <= (
                    pull_request_review_comment_event["sequence"]
                    if pull_request_review_comment_event is not None
                    else pull_request_comment_event["sequence"]
                )
                or pull_request_rejected_review_event["delivery_id"]
                == pull_request_comment_event["delivery_id"]
                or pull_request_rejected_review_event["payload_state"] != "open"
                or pull_request_rejected_review_event["payload_merged"] is not False
                or pull_request_rejected_review_event["head_sha"] != candidate_head_sha
                or pull_request_rejected_review_event["base_sha"] != opened_base_sha
                or pull_request_rejected_review_event["review_type"]
                != "pull_request_review_rejected"
            ):
                raise RuntimeError("Forgejo rejected-review event differs from its API readback")

        if approve_review:
            stage = "deliver_pull_request_approved_review_event"
            pull_request_approved_review_response, _ = owner_api.request(
                "POST",
                f"{pull_request_path}/reviews",
                {
                    "event": "APPROVED",
                    "body": "Independent local review approves this candidate.",
                    "commit_id": candidate_head_sha,
                },
            )
            pull_request_approved_review_http_status = owner_api.last_status
            approved_review_record_count = comment_record_count + 1 + int(
                pull_request_review_comment
            )
            records_after_approved_review = wait_for_journal_count(
                journal_path, approved_review_record_count
            )
            if len(records_after_approved_review) != approved_review_record_count:
                raise RuntimeError("Forgejo did not emit exactly one approved-review event")
            pull_request_approved_review_event = parse_live_pull_request_delivery_record(
                records_after_approved_review[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="reviewed",
                expected_event="pull_request_approved",
                expected_merged=False,
            )
            preceding_review_event = (
                pull_request_review_comment_event
                if pull_request_review_comment_event is not None
                else pull_request_comment_event
            )
            if (
                pull_request_approved_review_http_status != 200
                or pull_request_approved_review_response.get("state") != "APPROVED"
                or pull_request_approved_review_response.get("commit_id") != candidate_head_sha
                or pull_request_approved_review_response.get("user", {}).get("login") != OWNER
                or pull_request_approved_review_event["sequence"]
                <= preceding_review_event["sequence"]
                or pull_request_approved_review_event["delivery_id"]
                in {
                    pull_request_comment_event["delivery_id"],
                    *(
                        [pull_request_review_comment_event["delivery_id"]]
                        if pull_request_review_comment_event is not None
                        else []
                    ),
                }
                or pull_request_approved_review_event["payload_state"] != "open"
                or pull_request_approved_review_event["payload_merged"] is not False
                or pull_request_approved_review_event["head_sha"] != candidate_head_sha
                or pull_request_approved_review_event["base_sha"] != opened_base_sha
                or pull_request_approved_review_event["review_type"]
                != "pull_request_review_approved"
            ):
                raise RuntimeError("Forgejo approved-review event differs from its API readback")

        if request_review:
            stage = "deliver_pull_request_review_request_event"
            reviewer_id = reader_account.get("id") if reader_account is not None else None
            if not isinstance(reviewer_id, int) or isinstance(reviewer_id, bool) or reviewer_id < 1:
                raise RuntimeError("Forgejo reader account did not provide a valid reviewer ID")
            owner_api.request(
                "POST",
                f"{pull_request_path}/requested_reviewers",
                {"reviewers": [READER]},
            )
            review_request_http_status = owner_api.last_status
            requested_pull_request, _ = owner_api.request("GET", pull_request_path)
            raw_requested_reviewers = requested_pull_request.get("requested_reviewers")
            if not isinstance(raw_requested_reviewers, list):
                raise RuntimeError("Forgejo PR readback omitted requested reviewers")
            requested_reviewers_readback = [
                {"id": reviewer.get("id"), "login": reviewer.get("login")}
                for reviewer in raw_requested_reviewers
                if isinstance(reviewer, dict)
            ]
            if (
                review_request_http_status != 201
                or not any(
                    reviewer["id"] == reviewer_id and reviewer["login"] == READER
                    for reviewer in requested_reviewers_readback
                )
            ):
                raise RuntimeError("Forgejo did not confirm the requested reviewer in its PR API readback")

            post_review_future_events = (
                1 + 2 * int(reopen_pull_request) + int(edit_pull_request_title)
            )
            review_request_record_count = (
                pre_trailing_delivery_count
                - post_review_future_events
                - int(remove_review_request)
            )
            records_after_review_request = wait_for_journal_count(
                journal_path, review_request_record_count
            )
            if len(records_after_review_request) != review_request_record_count:
                raise RuntimeError("Forgejo did not emit exactly one review-request event")
            pull_request_review_request_event = parse_live_pull_request_delivery_record(
                records_after_review_request[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="review_requested",
                expected_event="pull_request",
                expected_requested_reviewer_login=READER,
                expected_merged=False,
            )
            if (
                pull_request_review_request_event["payload_state"] != "open"
                or pull_request_review_request_event["payload_merged"] is not False
                or pull_request_review_request_event["head_sha"] != candidate_head_sha
                or pull_request_review_request_event["base_sha"] != opened_base_sha
                or pull_request_review_request_event["requested_reviewer_id"] != reviewer_id
            ):
                raise RuntimeError("Forgejo review-request event differs from the canonical PR readback")

        if remove_review_request:
            stage = "deliver_pull_request_review_request_removed_event"
            owner_api.request(
                "DELETE",
                f"{pull_request_path}/requested_reviewers",
                {"reviewers": [READER]},
            )
            review_request_removal_http_status = owner_api.last_status
            updated_pull_request, _ = owner_api.request("GET", pull_request_path)
            raw_reviewers_after_removal = updated_pull_request.get("requested_reviewers")
            if not isinstance(raw_reviewers_after_removal, list):
                raise RuntimeError("Forgejo PR readback omitted reviewers after request removal")
            requested_reviewers_after_removal_readback = [
                {"id": reviewer.get("id"), "login": reviewer.get("login")}
                for reviewer in raw_reviewers_after_removal
                if isinstance(reviewer, dict)
            ]
            if (
                review_request_removal_http_status != 204
                or any(
                    reviewer["id"] == reviewer_id and reviewer["login"] == READER
                    for reviewer in requested_reviewers_after_removal_readback
                )
            ):
                raise RuntimeError("Forgejo did not confirm review-request removal in its PR API readback")

            removal_record_count = pre_trailing_delivery_count - post_review_future_events
            records_after_review_request_removal = wait_for_journal_count(
                journal_path, removal_record_count
            )
            if len(records_after_review_request_removal) != removal_record_count:
                raise RuntimeError("Forgejo did not emit exactly one review-request removal event")
            pull_request_review_request_removed_event = parse_live_pull_request_delivery_record(
                records_after_review_request_removal[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="review_request_removed",
                expected_event="pull_request",
                expected_requested_reviewer_login=READER,
                expected_merged=False,
            )
            if (
                pull_request_review_request_event is None
                or pull_request_review_request_removed_event["payload_state"] != "open"
                or pull_request_review_request_removed_event["payload_merged"] is not False
                or pull_request_review_request_removed_event["head_sha"] != candidate_head_sha
                or pull_request_review_request_removed_event["base_sha"] != opened_base_sha
                or pull_request_review_request_removed_event["requested_reviewer_id"] != reviewer_id
            ):
                raise RuntimeError("Forgejo review-request removal event differs from its API readback")

        if edit_pull_request_title:
            stage = "deliver_pull_request_title_edited_event"
            original_pull_request_title = "Exercise emitted pull-request webhooks"
            edited_pull_request_title = "Exercise emitted pull-request webhooks (edited)"
            owner_api.request("PATCH", pull_request_path, {"title": edited_pull_request_title})
            pull_request_title_edit_http_status = owner_api.last_status
            edited_pull_request, _ = owner_api.request("GET", pull_request_path)
            if (
                pull_request_title_edit_http_status != 201
                or edited_pull_request.get("title") != edited_pull_request_title
                or edited_pull_request.get("state") != "open"
                or edited_pull_request.get("merged") is not False
                or edited_pull_request.get("head", {}).get("sha") != candidate_head_sha
                or edited_pull_request.get("base", {}).get("sha") != opened_base_sha
            ):
                raise RuntimeError("Forgejo did not confirm the PR title edit and unchanged commits")

            title_edit_record_count = (
                pre_trailing_delivery_count - 1 - 2 * int(reopen_pull_request)
            )
            records_after_title_edit = wait_for_journal_count(
                journal_path, title_edit_record_count
            )
            if len(records_after_title_edit) != title_edit_record_count:
                raise RuntimeError("Forgejo did not emit exactly one pull-request title edit event")
            pull_request_title_edit_event = parse_live_pull_request_delivery_record(
                records_after_title_edit[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="edited",
                expected_event="pull_request",
                expected_title_from=original_pull_request_title,
                expected_merged=False,
            )
            if (
                pull_request_title_edit_event["payload_title"] != edited_pull_request_title
                or pull_request_title_edit_event["payload_state"] != "open"
                or pull_request_title_edit_event["head_sha"] != candidate_head_sha
                or pull_request_title_edit_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError("Forgejo title-edit webhook differs from the API readback")

        if reopen_pull_request:
            stage = "deliver_pull_request_close_before_reopen_event"
            owner_api.request("PATCH", pull_request_path, {"state": "closed"})
            close_before_reopen_status = owner_api.last_status
            closed_before_reopen, _ = owner_api.request("GET", pull_request_path)
            if (
                close_before_reopen_status != 201
                or closed_before_reopen.get("state") != "closed"
                or closed_before_reopen.get("merged") is not False
                or closed_before_reopen.get("head", {}).get("sha") != candidate_head_sha
            ):
                raise RuntimeError("Forgejo did not confirm the unmerged PR closure before reopen")

            records_before_reopen = wait_for_journal_count(
                journal_path, pre_trailing_delivery_count - 2
            )
            if len(records_before_reopen) != pre_trailing_delivery_count - 2:
                raise RuntimeError("Forgejo did not emit exactly one PR close event before reopen")
            pull_request_pre_reopen_closed_event = parse_live_pull_request_delivery_record(
                records_before_reopen[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="closed",
                expected_merged=False,
            )
            if (
                pull_request_pre_reopen_closed_event["payload_state"] != "closed"
                or pull_request_pre_reopen_closed_event["head_sha"] != candidate_head_sha
                or pull_request_pre_reopen_closed_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError("Forgejo pre-reopen close event differs from the candidate PR")

            stage = "deliver_pull_request_reopened_event"
            owner_api.request("PATCH", pull_request_path, {"state": "open"})
            reopen_http_status = owner_api.last_status
            reopened_pull_request, _ = owner_api.request("GET", pull_request_path)
            if (
                reopen_http_status != 201
                or reopened_pull_request.get("state") != "open"
                or reopened_pull_request.get("merged") is not False
                or reopened_pull_request.get("head", {}).get("sha") != candidate_head_sha
            ):
                raise RuntimeError("Forgejo did not confirm the unmerged PR reopen")

            records_after_reopen = wait_for_journal_count(
                journal_path, pre_trailing_delivery_count - 1
            )
            if len(records_after_reopen) != pre_trailing_delivery_count - 1:
                raise RuntimeError("Forgejo did not emit exactly one PR reopen event")
            pull_request_reopened_event = parse_live_pull_request_delivery_record(
                records_after_reopen[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="reopened",
                expected_merged=False,
            )
            if (
                pull_request_pre_reopen_closed_event["delivery_id"]
                == pull_request_reopened_event["delivery_id"]
                or pull_request_pre_reopen_closed_event["sequence"]
                >= pull_request_reopened_event["sequence"]
                or pull_request_reopened_event["payload_state"] != "open"
                or pull_request_reopened_event["head_sha"] != candidate_head_sha
                or pull_request_reopened_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError("Forgejo reopen event is duplicated, out of order, or stale")

        if change_pull_request_labels:
            stage = "deliver_pull_request_label_updated_event"
            pull_request_label_name = "agent-forge-e2e-live-label"
            created_label, _ = owner_api.request(
                "POST",
                f"/repos/{OWNER}/{REPOSITORY}/labels",
                {
                    "name": pull_request_label_name,
                    "color": "6272a4",
                    "description": "Disposable local PR label webhook fixture",
                },
            )
            pull_request_label_create_status = owner_api.last_status
            pull_request_label_id = created_label.get("id") if isinstance(created_label, dict) else None
            if (
                pull_request_label_create_status != 201
                or type(pull_request_label_id) is not int
                or pull_request_label_id < 1
                or created_label.get("name") != pull_request_label_name
            ):
                raise RuntimeError("Forgejo did not confirm creation of the disposable PR label")

            records_before_label_update = read_journal_records(journal_path)
            owner_api.request(
                "POST",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{pull_request_number}/labels",
                {"labels": [pull_request_label_name]},
            )
            pull_request_label_add_status = owner_api.last_status
            pull_request_after_label_add, _ = owner_api.request("GET", pull_request_path)
            labels_after_add = normalize_live_pull_request_labels(
                pull_request_after_label_add.get("labels")
                if isinstance(pull_request_after_label_add, dict)
                else None
            )
            if (
                pull_request_label_add_status != 200
                or {"id": pull_request_label_id, "name": pull_request_label_name}
                not in labels_after_add
                or pull_request_after_label_add.get("head", {}).get("sha") != candidate_head_sha
                or pull_request_after_label_add.get("base", {}).get("sha") != opened_base_sha
            ):
                raise RuntimeError("Forgejo PR API did not confirm the added label without changing commits")

            records_after_label_update = wait_for_journal_count(
                journal_path, len(records_before_label_update) + 1
            )
            if len(records_after_label_update) != len(records_before_label_update) + 1:
                raise RuntimeError("Forgejo did not emit exactly one pull_request/label_updated event")
            update_record = records_after_label_update[-1]
            update_envelope = update_record.get("payload") if isinstance(update_record, dict) else None
            update_payload = update_envelope.get("payload") if isinstance(update_envelope, dict) else None
            update_pull_request = (
                update_payload.get("pull_request") if isinstance(update_payload, dict) else None
            )
            update_payload_labels = normalize_live_pull_request_labels(
                update_pull_request.get("labels") if isinstance(update_pull_request, dict) else None
            )
            pull_request_label_update_event = parse_live_pull_request_delivery_record(
                update_record,
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="label_updated",
                expected_event="pull_request",
                expected_merged=False,
            )
            if (
                update_payload_labels != labels_after_add
                or {"id": pull_request_label_id, "name": pull_request_label_name}
                not in update_payload_labels
                or pull_request_label_update_event["payload_state"] != "open"
                or pull_request_label_update_event["head_sha"] != candidate_head_sha
                or pull_request_label_update_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError("Forgejo label_updated event differs from its PR payload/API readback")
            pull_request_label_events.append(pull_request_label_update_event)
            pull_request_label_event_payloads.append(update_payload_labels)
            pull_request_label_api_readbacks.append(labels_after_add)

            stage = "deliver_pull_request_label_cleared_event"
            records_before_label_clear = read_journal_records(journal_path)
            owner_api.request(
                "DELETE",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{pull_request_number}/labels",
                {},
            )
            pull_request_label_clear_status = owner_api.last_status
            pull_request_after_label_clear, _ = owner_api.request("GET", pull_request_path)
            labels_after_clear = normalize_live_pull_request_labels(
                pull_request_after_label_clear.get("labels")
                if isinstance(pull_request_after_label_clear, dict)
                else None
            )
            if (
                pull_request_label_clear_status != 204
                or labels_after_clear
                or pull_request_after_label_clear.get("head", {}).get("sha") != candidate_head_sha
                or pull_request_after_label_clear.get("base", {}).get("sha") != opened_base_sha
            ):
                raise RuntimeError("Forgejo PR API did not confirm clearing all labels")

            records_after_label_clear = wait_for_journal_count(
                journal_path, len(records_before_label_clear) + 1
            )
            if len(records_after_label_clear) != len(records_before_label_clear) + 1:
                raise RuntimeError("Forgejo did not emit exactly one pull_request/label_cleared event")
            clear_record = records_after_label_clear[-1]
            clear_envelope = clear_record.get("payload") if isinstance(clear_record, dict) else None
            clear_payload = clear_envelope.get("payload") if isinstance(clear_envelope, dict) else None
            clear_pull_request = (
                clear_payload.get("pull_request") if isinstance(clear_payload, dict) else None
            )
            clear_payload_labels = normalize_live_pull_request_labels(
                clear_pull_request.get("labels") if isinstance(clear_pull_request, dict) else None
            )
            pull_request_label_clear_event = parse_live_pull_request_delivery_record(
                clear_record,
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="label_cleared",
                expected_event="pull_request",
                expected_merged=False,
            )
            validate_live_pull_request_label_clear_snapshot(
                clear_payload_labels=clear_payload_labels,
                update_payload_labels=update_payload_labels,
                api_labels_after_clear=labels_after_clear,
                expected_label_id=pull_request_label_id,
                expected_label_name=pull_request_label_name,
            )
            if (
                pull_request_label_clear_event["payload_state"] != "open"
                or pull_request_label_clear_event["head_sha"] != candidate_head_sha
                or pull_request_label_clear_event["base_sha"] != opened_base_sha
                or pull_request_label_clear_event["sequence"]
                <= pull_request_label_update_event["sequence"]
                or pull_request_label_clear_event["delivery_id"]
                == pull_request_label_update_event["delivery_id"]
            ):
                raise RuntimeError("Forgejo label_cleared event is stale, duplicated, or malformed")
            pull_request_label_events.append(pull_request_label_clear_event)
            pull_request_label_event_payloads.append(clear_payload_labels)
            pull_request_label_api_readbacks.append(labels_after_clear)
            expected_delivery_count += 2
            pre_trailing_delivery_count += 2

        if change_pull_request_assignees:
            stage = "deliver_pull_request_assigned_event"
            assignee_id = reader_account.get("id") if isinstance(reader_account, dict) else None
            if type(assignee_id) is not int or assignee_id < 1:
                raise RuntimeError("Forgejo did not return a valid read-only PR assignee identity")
            expected_assignee = {"id": assignee_id, "login": READER}
            records_before_pull_request_assignee_update = read_journal_records(journal_path)
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{pull_request_number}",
                {"assignees": [READER]},
            )
            pull_request_assignee_add_status = owner_api.last_status
            pull_request_after_assignee_add, _ = owner_api.request("GET", pull_request_path)
            pull_request_assignees_after_add = normalize_live_issue_assignees(
                pull_request_after_assignee_add.get("assignees")
                if isinstance(pull_request_after_assignee_add, dict)
                else None
            )
            if (
                not isinstance(pull_request_after_assignee_add, dict)
                or
                pull_request_assignee_add_status != 201
                or pull_request_assignees_after_add != [expected_assignee]
                or pull_request_after_assignee_add.get("state") != "open"
                or pull_request_after_assignee_add.get("head", {}).get("sha") != candidate_head_sha
                or pull_request_after_assignee_add.get("base", {}).get("sha") != opened_base_sha
            ):
                raise RuntimeError("Forgejo PR API did not confirm the disposable assignment")

            records_after_pull_request_assignee_update = wait_for_journal_count(
                journal_path, len(records_before_pull_request_assignee_update) + 1
            )
            if len(records_after_pull_request_assignee_update) != len(
                records_before_pull_request_assignee_update
            ) + 1:
                raise RuntimeError("Forgejo did not emit exactly one pull_request/assigned event")
            pull_request_assigned_event = parse_live_pull_request_delivery_record(
                records_after_pull_request_assignee_update[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="assigned",
                expected_event="pull_request",
                expected_merged=False,
            )
            previous_delivery_sequence = records_before_pull_request_assignee_update[-1].get(
                "sequence"
            ) if records_before_pull_request_assignee_update else None
            if (
                pull_request_assigned_event["payload_assignees"] != pull_request_assignees_after_add
                or expected_assignee not in (pull_request_assigned_event["payload_assignees"] or [])
                or type(previous_delivery_sequence) is not int
                or pull_request_assigned_event["sequence"] <= previous_delivery_sequence
                or pull_request_assigned_event["payload_state"] != "open"
                or pull_request_assigned_event["head_sha"] != candidate_head_sha
                or pull_request_assigned_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError("Forgejo pull_request/assigned event differs from its API readback")
            pull_request_assignee_events.append(pull_request_assigned_event)
            pull_request_assignee_event_payloads.append(
                pull_request_assigned_event["payload_assignees"]
            )
            pull_request_assignee_api_readbacks.append(pull_request_assignees_after_add)

            stage = "deliver_pull_request_unassigned_event"
            records_before_pull_request_assignee_clear = read_journal_records(journal_path)
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{pull_request_number}",
                {"assignees": []},
            )
            pull_request_assignee_clear_status = owner_api.last_status
            pull_request_after_assignee_clear, _ = owner_api.request("GET", pull_request_path)
            pull_request_assignees_after_clear = normalize_live_issue_assignees(
                pull_request_after_assignee_clear.get("assignees")
                if isinstance(pull_request_after_assignee_clear, dict)
                else None
            )
            if (
                not isinstance(pull_request_after_assignee_clear, dict)
                or
                pull_request_assignee_clear_status != 201
                or pull_request_after_assignee_clear.get("state") != "open"
                or pull_request_after_assignee_clear.get("head", {}).get("sha") != candidate_head_sha
                or pull_request_after_assignee_clear.get("base", {}).get("sha") != opened_base_sha
            ):
                raise RuntimeError("Forgejo PR API did not confirm clearing assignees")

            records_after_pull_request_assignee_clear = wait_for_journal_count(
                journal_path, len(records_before_pull_request_assignee_clear) + 1
            )
            if len(records_after_pull_request_assignee_clear) != len(
                records_before_pull_request_assignee_clear
            ) + 1:
                raise RuntimeError("Forgejo did not emit exactly one pull_request/unassigned event")
            pull_request_unassigned_event = parse_live_pull_request_delivery_record(
                records_after_pull_request_assignee_clear[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="unassigned",
                expected_event="pull_request",
                expected_merged=False,
            )
            try:
                pull_request_assignee_clear_api_verdict = validate_live_issue_assignee_clear_snapshot(
                    clear_payload_assignees=pull_request_unassigned_event["payload_assignees"],
                    update_payload_assignees=pull_request_assigned_event["payload_assignees"],
                    api_assignees_after_clear=pull_request_assignees_after_clear,
                    expected_assignee=expected_assignee,
                )
            except ValueError as error:
                raise RuntimeError(
                    "Forgejo PR assignee clear disagrees with its prior event/API snapshot: "
                    f"{error}"
                ) from None
            if (
                pull_request_unassigned_event["sequence"] <= pull_request_assigned_event["sequence"]
                or pull_request_unassigned_event["delivery_id"]
                == pull_request_assigned_event["delivery_id"]
                or pull_request_unassigned_event["payload_state"] != "open"
                or pull_request_unassigned_event["head_sha"] != candidate_head_sha
                or pull_request_unassigned_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError("Forgejo pull_request/unassigned event is stale or duplicated")
            pull_request_assignee_events.append(pull_request_unassigned_event)
            pull_request_assignee_event_payloads.append(
                pull_request_unassigned_event["payload_assignees"]
            )
            pull_request_assignee_api_readbacks.append(pull_request_assignees_after_clear)
            expected_delivery_count += 2
            pre_trailing_delivery_count += 2

        if change_pull_request_milestones:
            stage = "deliver_pull_request_milestoned_event"
            pull_request_milestone_title = "agent-forge-e2e-live-pr-milestone"
            created_milestone, _ = owner_api.request(
                "POST",
                f"/repos/{OWNER}/{REPOSITORY}/milestones",
                {
                    "title": pull_request_milestone_title,
                    "description": "Disposable local pull-request milestone webhook fixture",
                },
            )
            pull_request_milestone_create_status = owner_api.last_status
            pull_request_milestone_id = (
                created_milestone.get("id") if isinstance(created_milestone, dict) else None
            )
            if (
                pull_request_milestone_create_status != 201
                or type(pull_request_milestone_id) is not int
                or pull_request_milestone_id < 1
                or created_milestone.get("title") != pull_request_milestone_title
                or created_milestone.get("state") != "open"
            ):
                raise RuntimeError("Forgejo did not confirm creation of the disposable PR milestone")

            records_before_pull_request_milestone_assign = read_journal_records(journal_path)
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{pull_request_number}",
                {"milestone": pull_request_milestone_id},
            )
            pull_request_milestone_assign_status = owner_api.last_status
            pull_request_after_milestone_assign, _ = owner_api.request(
                "GET", pull_request_path
            )
            if (
                not isinstance(pull_request_after_milestone_assign, dict)
                or "milestone" not in pull_request_after_milestone_assign
            ):
                raise RuntimeError("Forgejo PR API omitted its milestone after assignment")
            pull_request_milestone_after_assign = normalize_live_issue_milestone(
                pull_request_after_milestone_assign["milestone"]
            )
            expected_pull_request_milestone = {
                "id": pull_request_milestone_id,
                "title": pull_request_milestone_title,
                "state": "open",
            }
            if (
                pull_request_milestone_assign_status != 201
                or pull_request_milestone_after_assign != expected_pull_request_milestone
                or pull_request_after_milestone_assign.get("state") != "open"
                or pull_request_after_milestone_assign.get("head", {}).get("sha")
                != candidate_head_sha
                or pull_request_after_milestone_assign.get("base", {}).get("sha")
                != opened_base_sha
            ):
                raise RuntimeError("Forgejo PR API did not confirm the assigned milestone")

            records_after_pull_request_milestone_assign = wait_for_journal_count(
                journal_path, len(records_before_pull_request_milestone_assign) + 1
            )
            if len(records_after_pull_request_milestone_assign) != len(
                records_before_pull_request_milestone_assign
            ) + 1:
                raise RuntimeError(
                    "Forgejo did not emit exactly one pull_request/milestoned event"
                )
            pull_request_milestoned_event = parse_live_pull_request_delivery_record(
                records_after_pull_request_milestone_assign[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="milestoned",
                expected_event="pull_request",
                expected_merged=False,
            )
            if (
                pull_request_milestoned_event["payload_milestone"]
                != pull_request_milestone_after_assign
                or pull_request_milestoned_event["payload_state"] != "open"
                or pull_request_milestoned_event["head_sha"] != candidate_head_sha
                or pull_request_milestoned_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError(
                    "Forgejo pull_request/milestoned event differs from current API readback"
                )
            pull_request_milestone_events.append(pull_request_milestoned_event)
            pull_request_milestone_event_payloads.append(
                pull_request_milestoned_event["payload_milestone"]
            )
            pull_request_milestone_api_readbacks.append(pull_request_milestone_after_assign)

            stage = "deliver_pull_request_demilestoned_event"
            records_before_pull_request_milestone_clear = read_journal_records(journal_path)
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/issues/{pull_request_number}",
                {"milestone": 0},
            )
            pull_request_milestone_clear_status = owner_api.last_status
            pull_request_after_milestone_clear, _ = owner_api.request("GET", pull_request_path)
            if (
                not isinstance(pull_request_after_milestone_clear, dict)
                or "milestone" not in pull_request_after_milestone_clear
            ):
                raise RuntimeError("Forgejo PR API omitted its milestone after clearing")
            pull_request_milestone_after_clear = normalize_live_issue_milestone(
                pull_request_after_milestone_clear["milestone"]
            )
            if (
                pull_request_milestone_clear_status != 201
                or pull_request_milestone_after_clear is not None
                or pull_request_after_milestone_clear.get("state") != "open"
                or pull_request_after_milestone_clear.get("head", {}).get("sha")
                != candidate_head_sha
                or pull_request_after_milestone_clear.get("base", {}).get("sha")
                != opened_base_sha
            ):
                raise RuntimeError("Forgejo PR API did not confirm clearing the milestone")

            records_after_pull_request_milestone_clear = wait_for_journal_count(
                journal_path, len(records_before_pull_request_milestone_clear) + 1
            )
            if len(records_after_pull_request_milestone_clear) != len(
                records_before_pull_request_milestone_clear
            ) + 1:
                raise RuntimeError(
                    "Forgejo did not emit exactly one pull_request/demilestoned event"
                )
            pull_request_demilestoned_event = parse_live_pull_request_delivery_record(
                records_after_pull_request_milestone_clear[-1],
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_action="demilestoned",
                expected_event="pull_request",
                expected_merged=False,
            )
            try:
                pull_request_milestone_clear_api_verdict = (
                    validate_live_pull_request_milestone_clear_snapshot(
                        clear_payload_milestone=pull_request_demilestoned_event[
                            "payload_milestone"
                        ],
                        update_payload_milestone=pull_request_milestoned_event[
                            "payload_milestone"
                        ],
                        api_milestone_after_clear=pull_request_milestone_after_clear,
                        expected_milestone_id=pull_request_milestone_id,
                        expected_milestone_title=pull_request_milestone_title,
                    )
                )
            except ValueError as error:
                raise RuntimeError(
                    "Forgejo PR milestone clear disagrees with its prior event/API snapshot: "
                    f"{error}"
                ) from None
            if (
                pull_request_demilestoned_event["sequence"]
                <= pull_request_milestoned_event["sequence"]
                or pull_request_demilestoned_event["delivery_id"]
                == pull_request_milestoned_event["delivery_id"]
                or pull_request_demilestoned_event["payload_state"] != "open"
                or pull_request_demilestoned_event["head_sha"] != candidate_head_sha
                or pull_request_demilestoned_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError(
                    "Forgejo pull_request/demilestoned event is stale or duplicated"
                )
            pull_request_milestone_events.append(pull_request_demilestoned_event)
            pull_request_milestone_event_payloads.append(
                pull_request_demilestoned_event["payload_milestone"]
            )
            pull_request_milestone_api_readbacks.append(pull_request_milestone_after_clear)
            expected_delivery_count += 2
            pre_trailing_delivery_count += 2

        stage = "deliver_closed_pull_request_event"
        if args.merge_pull_request:
            owner_api.request(
                "POST",
                f"{pull_request_path}/merge",
                {"Do": "merge", "head_commit_id": candidate_head_sha},
            )
            merge_http_status = owner_api.last_status
            merged_pull_request, _ = owner_api.request("GET", pull_request_path)
            merge_commit_sha = merged_pull_request.get("merge_commit_sha")
            if (
                merge_http_status != 200
                or merged_pull_request.get("state") != "closed"
                or merged_pull_request.get("merged") is not True
                or not isinstance(merge_commit_sha, str)
                or not COMMIT_SHA_RE.fullmatch(merge_commit_sha)
            ):
                raise RuntimeError("Forgejo did not confirm the local PR merge")
            merge_response = {
                "http_status": merge_http_status,
                "merge_commit_sha": merge_commit_sha,
                "response_body": "empty; confirmed by PR readback",
            }
        else:
            owner_api.request("PATCH", pull_request_path, {"state": "closed"})
        if ordinary_issue_comment:
            comment_body = "Local ordinary issue-comment webhook integration probe."
            for action_index, action in enumerate(ordinary_issue_comment_actions):
                if action == "created":
                    stage = "deliver_ordinary_issue_comment_created_event"
                    comment, _ = owner_api.request(
                        "POST",
                        f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}/comments",
                        {"body": comment_body},
                    )
                    ordinary_issue_comment_id = comment.get("id")
                    if (
                        owner_api.last_status != 201
                        or not isinstance(ordinary_issue_comment_id, int)
                        or isinstance(ordinary_issue_comment_id, bool)
                        or ordinary_issue_comment_id < 1
                        or comment.get("body") != comment_body
                    ):
                        raise RuntimeError(
                            "Forgejo did not confirm creation of the ordinary issue comment"
                        )
                elif action == "edited":
                    stage = "deliver_ordinary_issue_comment_edited_event"
                    assert ordinary_issue_comment_id is not None
                    comment_body = "Edited local ordinary issue-comment webhook probe."
                    comment, _ = owner_api.request(
                        "PATCH",
                        f"/repos/{OWNER}/{REPOSITORY}/issues/comments/{ordinary_issue_comment_id}",
                        {"body": comment_body},
                    )
                    if (
                        owner_api.last_status != 200
                        or comment.get("id") != ordinary_issue_comment_id
                        or comment.get("body") != comment_body
                    ):
                        raise RuntimeError(
                            "Forgejo did not confirm editing the ordinary issue comment"
                        )
                elif action == "deleted":
                    stage = "deliver_ordinary_issue_comment_deleted_event"
                    assert ordinary_issue_comment_id is not None
                    owner_api.request(
                        "DELETE",
                        f"/repos/{OWNER}/{REPOSITORY}/issues/comments/{ordinary_issue_comment_id}",
                    )
                    if owner_api.last_status != 204:
                        raise RuntimeError(
                            "Forgejo did not confirm deleting the ordinary issue comment"
                        )
                    comment_readback_path = (
                        f"/repos/{OWNER}/{REPOSITORY}/issues/comments/{ordinary_issue_comment_id}"
                    )
                    owner_api.request(
                        "GET",
                        comment_readback_path,
                        expected_http_error_statuses=(404,),
                    )
                    ordinary_issue_comment_delete_readback = (
                        validate_issue_comment_delete_readback(
                            owner_api.last_status, ordinary_issue_comment_id
                        )
                    )
                expected_count = pre_trailing_delivery_count + action_index + 1
                action_records = wait_for_journal_count(journal_path, expected_count)
                if len(action_records) != expected_count:
                    raise RuntimeError(
                        "Forgejo delivered an unexpected number of ordinary issue-comment events"
                    )
                assert ordinary_issue_comment_id is not None
                parsed = parse_live_issue_comment_delivery_record(
                    action_records[-1],
                    owner=OWNER,
                    repository=REPOSITORY,
                    issue_number=issue_number,
                    expected_action=action,
                    expected_comment_id=ordinary_issue_comment_id,
                )
                if ordinary_issue_comment_events and (
                    parsed["sequence"] <= ordinary_issue_comment_events[-1]["sequence"]
                    or parsed["delivery_id"]
                    == ordinary_issue_comment_events[-1]["delivery_id"]
                ):
                    raise RuntimeError(
                        "Forgejo ordinary issue-comment events are duplicated or out of order"
                    )
                ordinary_issue_comment_events.append(parsed)
                status, reconciliation = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    parsed["delivery_id"],
                    reader_token,
                )
                validate_ordinary_issue_comment_reconciliation(
                    status,
                    parsed,
                    reconciliation,
                    owner=OWNER,
                    repository=REPOSITORY,
                    issue_number=issue_number,
                    expected_comment_state=("absent" if action == "deleted" else "present"),
                    expected_body=(None if action == "deleted" else comment_body),
                )
                ordinary_issue_comment_statuses.append(status)
                ordinary_issue_comment_reconciliations.append(reconciliation)
            ordinary_issue_comment_event = ordinary_issue_comment_events[0]
        all_records = wait_for_journal_count(journal_path, expected_delivery_count)
        if len(all_records) != expected_delivery_count:
            raise RuntimeError("Forgejo delivered an unexpected number of issue and PR events")
        pull_request_closed_event = parse_live_pull_request_delivery_record(
            all_records[-(ordinary_issue_comment_event_count + 1)],
            owner=OWNER,
            repository=REPOSITORY,
            pull_request_number=pull_request_number,
            expected_action="closed",
            expected_merged=args.merge_pull_request,
        )
        if ordinary_issue_comment:
            for event, action, record in zip(
                ordinary_issue_comment_events,
                ordinary_issue_comment_actions,
                all_records[-ordinary_issue_comment_event_count:],
                strict=True,
            ):
                parsed = parse_live_issue_comment_delivery_record(
                    record,
                    owner=OWNER,
                    repository=REPOSITORY,
                    issue_number=issue_number,
                    expected_action=action,
                    expected_comment_id=ordinary_issue_comment_id,
                )
                if event != parsed:
                    raise RuntimeError("Forgejo ordinary issue-comment event changed after receipt")
            if (
                ordinary_issue_comment_events[0]["sequence"]
                <= pull_request_closed_event["sequence"]
                or ordinary_issue_comment_events[0]["delivery_id"]
                == pull_request_closed_event["delivery_id"]
            ):
                raise RuntimeError("Forgejo ordinary issue comment was not emitted after PR closure")
        if (
            pull_request_open_event["delivery_id"] == pull_request_closed_event["delivery_id"]
            or pull_request_open_event["sequence"] >= pull_request_comment_event["sequence"]
            or (
                pull_request_sync_event is not None
                and (
                    pull_request_sync_event["delivery_id"] in {
                        pull_request_open_event["delivery_id"],
                        pull_request_comment_event["delivery_id"],
                        pull_request_closed_event["delivery_id"],
                    }
                    or pull_request_open_event["sequence"] >= pull_request_sync_event["sequence"]
                    or pull_request_sync_event["sequence"] >= pull_request_comment_event["sequence"]
                )
            )
            or pull_request_comment_event["delivery_id"] == pull_request_closed_event["delivery_id"]
            or pull_request_comment_event["sequence"] >= pull_request_closed_event["sequence"]
            or (
                pull_request_review_comment_event is not None
                and (
                    pull_request_review_comment_event["delivery_id"]
                    in {
                        pull_request_open_event["delivery_id"],
                        pull_request_comment_event["delivery_id"],
                        pull_request_closed_event["delivery_id"],
                    }
                    or pull_request_review_comment_event["sequence"]
                    <= pull_request_comment_event["sequence"]
                    or pull_request_review_comment_event["sequence"]
                    >= pull_request_closed_event["sequence"]
                )
            )
            or pull_request_closed_event["payload_state"] != "closed"
            or pull_request_closed_event["payload_merged"] is not args.merge_pull_request
            or pull_request_closed_event["head_sha"] != candidate_head_sha
            or (
                not args.merge_pull_request
                and pull_request_closed_event["base_sha"] != opened_base_sha
            )
            or (
                args.merge_pull_request
                and pull_request_closed_event.get("merge_commit_sha")
                != merge_response.get("merge_commit_sha")
            )
        ):
            raise RuntimeError("Forgejo PR transition events do not preserve the same candidate commits")
        if reopen_pull_request:
            if (
                pull_request_pre_reopen_closed_event is None
                or pull_request_reopened_event is None
            ):
                raise RuntimeError("Forgejo reopen scenario omitted a lifecycle event")
            lifecycle_events = [
                pull_request_open_event,
                *([pull_request_sync_event] if pull_request_sync_event is not None else []),
                pull_request_comment_event,
                *(
                    [pull_request_review_comment_event]
                    if pull_request_review_comment_event is not None
                    else []
                ),
                *(
                    [pull_request_rejected_review_event]
                    if pull_request_rejected_review_event is not None
                    else []
                ),
                *(
                    [pull_request_approved_review_event]
                    if pull_request_approved_review_event is not None
                    else []
                ),
                *(
                    [pull_request_review_request_event]
                    if pull_request_review_request_event is not None
                    else []
                ),
                *(
                    [pull_request_review_request_removed_event]
                    if pull_request_review_request_removed_event is not None
                    else []
                ),
                pull_request_pre_reopen_closed_event,
                pull_request_reopened_event,
                pull_request_closed_event,
            ]
            if (
                len({event["delivery_id"] for event in lifecycle_events})
                != len(lifecycle_events)
                or [event["sequence"] for event in lifecycle_events]
                != sorted(event["sequence"] for event in lifecycle_events)
                or pull_request_pre_reopen_closed_event["action"] != "closed"
                or pull_request_pre_reopen_closed_event["payload_state"] != "closed"
                or pull_request_pre_reopen_closed_event["payload_merged"] is not False
                or pull_request_reopened_event["action"] != "reopened"
                or pull_request_reopened_event["payload_state"] != "open"
                or pull_request_reopened_event["payload_merged"] is not False
                or pull_request_reopened_event["head_sha"] != candidate_head_sha
                or pull_request_reopened_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError("Forgejo PR close/reopen lifecycle is not unique and ordered")
        if pull_request_review_request_event is not None:
            preceding_pr_events = [
                pull_request_open_event,
                *([pull_request_sync_event] if pull_request_sync_event is not None else []),
                pull_request_comment_event,
                *(
                    [pull_request_review_comment_event]
                    if pull_request_review_comment_event is not None
                    else []
                ),
                *(
                    [pull_request_rejected_review_event]
                    if pull_request_rejected_review_event is not None
                    else []
                ),
                *(
                    [pull_request_approved_review_event]
                    if pull_request_approved_review_event is not None
                    else []
                ),
            ]
            if (
                pull_request_review_request_event["delivery_id"]
                in {event["delivery_id"] for event in preceding_pr_events}
                or pull_request_review_request_event["sequence"]
                <= max(event["sequence"] for event in preceding_pr_events)
                or pull_request_review_request_event["delivery_id"]
                == pull_request_closed_event["delivery_id"]
                or pull_request_review_request_event["sequence"]
                >= pull_request_closed_event["sequence"]
            ):
                raise RuntimeError("Forgejo review-request delivery is duplicated or out of order")
        if pull_request_review_request_removed_event is not None:
            if (
                pull_request_review_request_event is None
                or pull_request_review_request_removed_event["delivery_id"]
                == pull_request_review_request_event["delivery_id"]
                or pull_request_review_request_removed_event["sequence"]
                <= pull_request_review_request_event["sequence"]
                or pull_request_review_request_removed_event["delivery_id"]
                == pull_request_closed_event["delivery_id"]
                or pull_request_review_request_removed_event["sequence"]
                >= pull_request_closed_event["sequence"]
                or pull_request_review_request_removed_event["requested_reviewer_id"]
                != pull_request_review_request_event["requested_reviewer_id"]
                or pull_request_review_request_removed_event["requested_reviewer_login"]
                != pull_request_review_request_event["requested_reviewer_login"]
            ):
                raise RuntimeError("Forgejo review-request removal delivery is duplicated or out of order")
        if pull_request_title_edit_event is not None:
            preceding_pr_events = [
                pull_request_open_event,
                *([pull_request_sync_event] if pull_request_sync_event is not None else []),
                pull_request_comment_event,
                *(
                    [pull_request_review_comment_event]
                    if pull_request_review_comment_event is not None
                    else []
                ),
                *(
                    [pull_request_rejected_review_event]
                    if pull_request_rejected_review_event is not None
                    else []
                ),
                *(
                    [pull_request_approved_review_event]
                    if pull_request_approved_review_event is not None
                    else []
                ),
                *(
                    [pull_request_review_request_event]
                    if pull_request_review_request_event is not None
                    else []
                ),
                *(
                    [pull_request_review_request_removed_event]
                    if pull_request_review_request_removed_event is not None
                    else []
                ),
            ]
            title_edit_boundary = (
                pull_request_pre_reopen_closed_event
                if reopen_pull_request
                else pull_request_closed_event
            )
            if (
                title_edit_boundary is None
                or pull_request_title_edit_event["delivery_id"]
                in {event["delivery_id"] for event in preceding_pr_events}
                or pull_request_title_edit_event["sequence"]
                <= max(event["sequence"] for event in preceding_pr_events)
                or pull_request_title_edit_event["delivery_id"]
                == title_edit_boundary["delivery_id"]
                or pull_request_title_edit_event["sequence"]
                >= title_edit_boundary["sequence"]
                or pull_request_title_edit_event["payload_state"] != "open"
                or pull_request_title_edit_event["payload_merged"] is not False
                or pull_request_title_edit_event["head_sha"] != candidate_head_sha
                or pull_request_title_edit_event["base_sha"] != opened_base_sha
            ):
                raise RuntimeError("Forgejo PR title-edit delivery is duplicated or out of order")

        stage = "read_forgejo_delivery_tasks"
        hook_tasks = wait_for_hook_tasks(state / "data/forgejo.db", hook_id, expected_delivery_count)
        all_events = [
            open_event,
            *issue_label_events,
            *issue_assignee_events,
            *issue_milestone_events,
            closed_event,
            *edited_events,
            *([issue_reopened_event] if issue_reopened_event is not None else []),
            pull_request_open_event,
            *([pull_request_sync_event] if pull_request_sync_event is not None else []),
            *(
                [pull_request_comment_created_event]
                if delete_pull_request_comment
                else []
            ),
            pull_request_comment_event,
            *(
                [pull_request_review_comment_event]
                if pull_request_review_comment_event is not None
                else []
            ),
            *(
                [pull_request_rejected_review_event]
                if pull_request_rejected_review_event is not None
                else []
            ),
            *(
                [pull_request_approved_review_event]
                if pull_request_approved_review_event is not None
                else []
            ),
            *(
                [pull_request_review_request_event]
                if pull_request_review_request_event is not None
                else []
            ),
            *(
                [pull_request_review_request_removed_event]
                if pull_request_review_request_removed_event is not None
                else []
            ),
            *(
                [pull_request_title_edit_event]
                if pull_request_title_edit_event is not None
                else []
            ),
            *(
                [pull_request_pre_reopen_closed_event]
                if pull_request_pre_reopen_closed_event is not None
                else []
            ),
            *(
                [pull_request_reopened_event]
                if pull_request_reopened_event is not None
                else []
            ),
            *pull_request_label_events,
            *pull_request_assignee_events,
            *pull_request_milestone_events,
            pull_request_closed_event,
            *ordinary_issue_comment_events,
        ]
        if len(hook_tasks) != expected_delivery_count:
            raise RuntimeError("Forgejo recorded an unexpected number of webhook task events")
        validate_hook_task_bindings(hook_tasks, all_events)

        stage = "redeliver_emitted_event_after_receiver_restart"
        journal_bytes_before_replay = journal_path.read_bytes()
        records_before_replay = read_journal_records(journal_path)
        if len(records_before_replay) != expected_delivery_count or records_before_replay != all_records:
            raise RuntimeError("receiver journal changed before duplicate redelivery")
        receiver_stop = kill_process_with_sigkill(receiver)
        addresses_after_first_stop = helpers.listener_addresses_for_port(receiver_port)
        receiver_stop_history.append(
            {
                "phase": "sigkill_before_duplicate_redelivery",
                "stop": receiver_stop,
                "listener_addresses_after_stop": addresses_after_first_stop,
                "listener_checked": True,
            }
        )
        if (
            not receiver_stop["stopped"]
            or receiver_stop["stop_signal"] != "SIGKILL"
            or addresses_after_first_stop
        ):
            raise RuntimeError("receiver was not stopped by SIGKILL before duplicate redelivery")
        receiver = None
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
        duplicate_event = select_duplicate_replay_event(
            open_event,
            pull_request_assignee_events if change_pull_request_assignees else [],
        )
        matching_duplicate_records = [
            record
            for record in all_records
            if isinstance(record.get("payload"), dict)
            and record["payload"].get("delivery_id") == duplicate_event["delivery_id"]
        ]
        if len(matching_duplicate_records) != 1:
            raise RuntimeError("selected duplicate replay event is missing or ambiguous in the journal")
        duplicate_envelope = matching_duplicate_records[0]["payload"]
        duplicate_payload = duplicate_envelope.get("payload")
        if not isinstance(duplicate_payload, dict):
            raise RuntimeError("captured Forgejo delivery payload is invalid")
        replay_status, replay_response = send_redelivered_webhook(
            receiver_address,
            webhook_secret.encode("ascii"),
            duplicate_event["delivery_id"],
            duplicate_event["event"],
            duplicate_payload,
        )
        records_after_replay = wait_for_journal_count(journal_path, expected_delivery_count)
        journal_bytes_after_replay = journal_path.read_bytes()
        if len(records_after_replay) != expected_delivery_count:
            raise RuntimeError("duplicate Forgejo redelivery changed the journal size")
        validate_duplicate_delivery_replay(
            duplicate_event,
            replay_status,
            replay_response,
            records_before_replay,
            records_after_replay,
        )
        validate_journal_snapshot_unchanged(
            journal_bytes_before_replay,
            journal_bytes_after_replay,
        )
        duplicate_replay = {
            "delivery_id": duplicate_event["delivery_id"],
            "event": duplicate_event["event"],
            "action": duplicate_event.get("action"),
            "sequence": duplicate_event["sequence"],
            "http_status": replay_status,
            "response": replay_response,
            "journal_records_before": len(records_before_replay),
            "journal_records_after": len(records_after_replay),
            "journal_bytes_before": len(journal_bytes_before_replay),
            "journal_bytes_after": len(journal_bytes_after_replay),
            "journal_sha256_before": hashlib.sha256(journal_bytes_before_replay).hexdigest(),
            "journal_sha256_after": hashlib.sha256(journal_bytes_after_replay).hexdigest(),
            "receiver_restarted_with_same_journal": True,
            "payload_source": "verified Forgejo-emitted receiver journal envelope",
            "payload_encoding": "JSON re-serialized from the journaled object; byte-for-byte original request body is not retained",
        }
        helpers.write_safe_json(
            evidence_dir / "duplicate-delivery-replay.json", duplicate_replay, tokens
        )

        stage = "reconcile_issue_and_pull_request_events"
        open_status, open_reconciliation = helpers.run_reconcile(
            reconcile_bin, state, journal_path, open_event["delivery_id"], reader_token
        )
        for event in issue_label_events:
            status, result = helpers.run_reconcile(
                reconcile_bin,
                state,
                journal_path,
                event["delivery_id"],
                reader_token,
            )
            issue_label_event_statuses.append(status)
            issue_label_reconciliations.append(result)
        for event in issue_assignee_events:
            status, result = helpers.run_reconcile(
                reconcile_bin,
                state,
                journal_path,
                event["delivery_id"],
                reader_token,
            )
            issue_assignee_event_statuses.append(status)
            issue_assignee_reconciliations.append(result)
        for event in issue_milestone_events:
            status, result = helpers.run_reconcile(
                reconcile_bin,
                state,
                journal_path,
                event["delivery_id"],
                reader_token,
            )
            issue_milestone_event_statuses.append(status)
            issue_milestone_reconciliations.append(result)
        closed_status, closed_reconciliation = helpers.run_reconcile(
            reconcile_bin, state, journal_path, closed_event["delivery_id"], reader_token
        )
        edited_reconciliations = []
        edited_statuses = []
        for event in edited_events:
            status, result = helpers.run_reconcile(
                reconcile_bin, state, journal_path, event["delivery_id"], reader_token
            )
            edited_statuses.append(status)
            edited_reconciliations.append(result)
        if issue_reopened_event is not None:
            issue_reopened_status, issue_reopened_reconciliation = helpers.run_reconcile(
                reconcile_bin,
                state,
                journal_path,
                issue_reopened_event["delivery_id"],
                reader_token,
            )
        if ordinary_issue_comment_events:
            ordinary_issue_comment_status = ordinary_issue_comment_statuses[0]
            ordinary_issue_comment_reconciliation = ordinary_issue_comment_reconciliations[0]
        if batch_pull_request_events:
            requested_pull_request_events = [
                pull_request_open_event,
                *([pull_request_sync_event] if pull_request_sync_event is not None else []),
                pull_request_comment_event,
                *(
                    [pull_request_review_comment_event]
                    if pull_request_review_comment_event is not None
                    else []
                ),
                *(
                    [pull_request_rejected_review_event]
                    if pull_request_rejected_review_event is not None
                    else []
                ),
                *(
                    [pull_request_approved_review_event]
                    if pull_request_approved_review_event is not None
                    else []
                ),
                *(
                    [pull_request_review_request_event]
                    if pull_request_review_request_event is not None
                    else []
                ),
                *(
                    [pull_request_review_request_removed_event]
                    if pull_request_review_request_removed_event is not None
                    else []
                ),
                *(
                    [pull_request_title_edit_event]
                    if pull_request_title_edit_event is not None
                    else []
                ),
                *(
                    [pull_request_pre_reopen_closed_event]
                    if pull_request_pre_reopen_closed_event is not None
                    else []
                ),
                *(
                    [pull_request_reopened_event]
                    if pull_request_reopened_event is not None
                    else []
                ),
                *pull_request_label_events,
                *pull_request_assignee_events,
                *pull_request_milestone_events,
                pull_request_closed_event,
            ]
            if any(not isinstance(event, dict) for event in requested_pull_request_events):
                raise RuntimeError("pull-request batch selection contains a missing event")
            batch_trace_path = (
                evidence_dir / "batch-connect-trace.log" if observe_batch_network else None
            )
            batch_status, pull_request_batch_reconciliation, remaining_reader_credential = (
                run_reconcile_batch(
                    reconcile_bin,
                    state,
                    journal_path,
                    [event["delivery_id"] for event in requested_pull_request_events],
                    reader_token,
                    receiver_attestation_public_key,
                    trace_path=batch_trace_path,
                    strace_binary=strace_binary,
                )
            )
            if remaining_reader_credential:
                raise RuntimeError("pull-request batch did not consume its private reader credential")
            if observe_batch_network:
                assert batch_trace_path is not None and strace_binary is not None
                trace_bytes = batch_trace_path.read_bytes()
                if len(trace_bytes) > 64 * 1024 or any(
                    token.encode("utf-8") in trace_bytes for token in tokens
                ):
                    raise RuntimeError("batch network trace exceeded limits or contained a credential")
                batch_network_observation = parse_loopback_connect_trace(
                    trace_bytes.decode("utf-8", errors="strict"),
                    expected_host="127.0.0.1",
                    expected_port=3130,
                )
                batch_network_observation["strace_version"] = strace_version
                batch_network_observation["strace_binary_sha256"] = helpers.sha256_file(
                    strace_binary
                )
                helpers.write_safe_json(
                    evidence_dir / "batch-network-observation.json",
                    batch_network_observation,
                    tokens,
                )
            checked_pull_request_results = validate_pull_request_reconcile_batch(
                batch_status,
                requested_pull_request_events,
                pull_request_batch_reconciliation,
                owner=OWNER,
                repository=REPOSITORY,
                pull_request_number=pull_request_number,
                expected_requested_reviewers=(
                    requested_reviewers_after_removal_readback
                    if remove_review_request
                    else requested_reviewers_readback
                    if request_review
                    else None
                ),
                expected_requested_reviewer_teams=requested_reviewer_teams_readback,
            )
            result_index = 0
            pull_request_open_status = batch_status
            pull_request_open_reconciliation = checked_pull_request_results[result_index]
            result_index += 1
            if pull_request_sync_event is not None:
                pull_request_sync_status = batch_status
                pull_request_sync_reconciliation = checked_pull_request_results[result_index]
                result_index += 1
            pull_request_comment_status = batch_status
            pull_request_comment_reconciliation = checked_pull_request_results[result_index]
            result_index += 1
            if pull_request_review_comment_event is not None:
                pull_request_review_comment_status = batch_status
                pull_request_review_comment_reconciliation = checked_pull_request_results[result_index]
                result_index += 1
            if pull_request_rejected_review_event is not None:
                pull_request_rejected_review_status = batch_status
                pull_request_rejected_review_reconciliation = checked_pull_request_results[result_index]
                result_index += 1
            if pull_request_approved_review_event is not None:
                pull_request_approved_review_status = batch_status
                pull_request_approved_review_reconciliation = checked_pull_request_results[result_index]
                result_index += 1
            if pull_request_review_request_event is not None:
                pull_request_review_request_status = batch_status
                pull_request_review_request_reconciliation = checked_pull_request_results[result_index]
                result_index += 1
            if pull_request_review_request_removed_event is not None:
                pull_request_review_request_removed_status = batch_status
                pull_request_review_request_removed_reconciliation = checked_pull_request_results[result_index]
                result_index += 1
            if pull_request_title_edit_event is not None:
                pull_request_title_edit_status = batch_status
                pull_request_title_edit_reconciliation = checked_pull_request_results[result_index]
                result_index += 1
            if pull_request_pre_reopen_closed_event is not None:
                pull_request_pre_reopen_closed_status = batch_status
                pull_request_pre_reopen_closed_reconciliation = checked_pull_request_results[
                    result_index
                ]
                result_index += 1
            if pull_request_reopened_event is not None:
                pull_request_reopened_status = batch_status
                pull_request_reopened_reconciliation = checked_pull_request_results[result_index]
                result_index += 1
            for _ in pull_request_label_events:
                pull_request_label_event_statuses.append(batch_status)
                pull_request_label_reconciliations.append(
                    checked_pull_request_results[result_index]
                )
                result_index += 1
            for _ in pull_request_assignee_events:
                pull_request_assignee_event_statuses.append(batch_status)
                pull_request_assignee_reconciliations.append(
                    checked_pull_request_results[result_index]
                )
                result_index += 1
            for _ in pull_request_milestone_events:
                pull_request_milestone_event_statuses.append(batch_status)
                pull_request_milestone_reconciliations.append(
                    checked_pull_request_results[result_index]
                )
                result_index += 1
            pull_request_closed_status = batch_status
            pull_request_closed_reconciliation = checked_pull_request_results[result_index]
        else:
            pull_request_open_status, pull_request_open_reconciliation = helpers.run_reconcile(
                reconcile_bin,
                state,
                journal_path,
                pull_request_open_event["delivery_id"],
                reader_token,
            )
            if pull_request_sync_event is not None:
                pull_request_sync_status, pull_request_sync_reconciliation = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    pull_request_sync_event["delivery_id"],
                    reader_token,
                )
            if not revoke_pull_request_comment_reader:
                pull_request_comment_status, pull_request_comment_reconciliation = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    pull_request_comment_event["delivery_id"],
                    reader_token,
                )
            if delete_pull_request_comment and not revoke_pull_request_comment_reader:
                assert pull_request_comment_created_event is not None
                (
                    pull_request_comment_created_status,
                    pull_request_comment_created_reconciliation,
                ) = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    pull_request_comment_created_event["delivery_id"],
                    reader_token,
                )
                validate_deleted_pull_request_comment_inconclusive_created_event(
                    pull_request_comment_created_status,
                    pull_request_comment_created_event,
                    pull_request_comment_created_reconciliation,
                )
            if pull_request_review_comment_event is not None:
                (
                    pull_request_review_comment_status,
                    pull_request_review_comment_reconciliation,
                ) = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    pull_request_review_comment_event["delivery_id"],
                    reader_token,
                )
            if pull_request_rejected_review_event is not None:
                pull_request_rejected_review_status, pull_request_rejected_review_reconciliation = (
                    helpers.run_reconcile(
                        reconcile_bin,
                        state,
                        journal_path,
                        pull_request_rejected_review_event["delivery_id"],
                        reader_token,
                    )
                )
            if pull_request_approved_review_event is not None:
                (
                    pull_request_approved_review_status,
                    pull_request_approved_review_reconciliation,
                ) = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    pull_request_approved_review_event["delivery_id"],
                    reader_token,
                )
            if pull_request_review_request_event is not None:
                (
                    pull_request_review_request_status,
                    pull_request_review_request_reconciliation,
                ) = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    pull_request_review_request_event["delivery_id"],
                    reader_token,
                )
            if pull_request_review_request_removed_event is not None:
                (
                    pull_request_review_request_removed_status,
                    pull_request_review_request_removed_reconciliation,
                ) = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    pull_request_review_request_removed_event["delivery_id"],
                    reader_token,
                )
            if pull_request_title_edit_event is not None:
                (
                    pull_request_title_edit_status,
                    pull_request_title_edit_reconciliation,
                ) = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    pull_request_title_edit_event["delivery_id"],
                    reader_token,
                )
            if pull_request_pre_reopen_closed_event is not None:
                (
                    pull_request_pre_reopen_closed_status,
                    pull_request_pre_reopen_closed_reconciliation,
                ) = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    pull_request_pre_reopen_closed_event["delivery_id"],
                    reader_token,
                )
            if pull_request_reopened_event is not None:
                pull_request_reopened_status, pull_request_reopened_reconciliation = (
                    helpers.run_reconcile(
                        reconcile_bin,
                        state,
                        journal_path,
                        pull_request_reopened_event["delivery_id"],
                        reader_token,
                    )
                )
            for event in pull_request_label_events:
                status, result = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    event["delivery_id"],
                    reader_token,
                )
                pull_request_label_event_statuses.append(status)
                pull_request_label_reconciliations.append(result)
            for event in pull_request_assignee_events:
                status, result = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    event["delivery_id"],
                    reader_token,
                )
                pull_request_assignee_event_statuses.append(status)
                pull_request_assignee_reconciliations.append(result)
            for event in pull_request_milestone_events:
                status, result = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    event["delivery_id"],
                    reader_token,
                )
                pull_request_milestone_event_statuses.append(status)
                pull_request_milestone_reconciliations.append(result)
            pull_request_closed_status, pull_request_closed_reconciliation = helpers.run_reconcile(
                reconcile_bin,
                state,
                journal_path,
                pull_request_closed_event["delivery_id"],
                reader_token,
            )
        for event, status, result in zip(
            pull_request_label_events,
            pull_request_label_event_statuses,
            pull_request_label_reconciliations,
            strict=True,
        ):
            validate_reconciliation_delivery_binding(event, result)
            resource = result.get("resource") if isinstance(result, dict) else None
            canonical_state = result.get("canonical_state") if isinstance(result, dict) else None
            current_labels_projection = (
                canonical_state.get("labels") if isinstance(canonical_state, dict) else None
            )
            if (
                status != 0
                or (not batch_pull_request_events and result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION)
                or result.get("verdict") != "current_state_read"
                or not isinstance(resource, dict)
                or resource.get("kind") != "pull_request"
                or resource.get("number") != pull_request_number
                or resource.get("owner") != OWNER
                or resource.get("repository") != REPOSITORY
                or not isinstance(canonical_state, dict)
                or not is_current_empty_label_projection(current_labels_projection)
            ):
                raise RuntimeError(
                    "label webhook did not reconcile to the current empty Forgejo PR label set"
                )
        for event, status, result in zip(
            pull_request_assignee_events,
            pull_request_assignee_event_statuses,
            pull_request_assignee_reconciliations,
            strict=True,
        ):
            validate_reconciliation_delivery_binding(event, result)
            resource = result.get("resource") if isinstance(result, dict) else None
            canonical_state = result.get("canonical_state") if isinstance(result, dict) else None
            assignees_projection = (
                canonical_state.get("assignees") if isinstance(canonical_state, dict) else None
            )
            if (
                status != 0
                or (not batch_pull_request_events and result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION)
                or result.get("verdict") != "current_state_read"
                or (not batch_pull_request_events and result.get("upstream_read_attempts") != 1)
                or not isinstance(resource, dict)
                or resource.get("kind") != "pull_request"
                or resource.get("number") != pull_request_number
                or resource.get("owner") != OWNER
                or resource.get("repository") != REPOSITORY
                or not isinstance(canonical_state, dict)
                or (
                    pull_request_assignee_clear_api_verdict == "known_empty"
                    and not is_current_empty_assignee_projection(assignees_projection)
                )
                or (
                    pull_request_assignee_clear_api_verdict == "unknown_null_field"
                    and assignees_projection is not None
                )
            ):
                raise RuntimeError(
                    "PR assignment event did not reconcile to the current Forgejo API assignee certainty"
                )
        for event, status, result in zip(
            pull_request_milestone_events,
            pull_request_milestone_event_statuses,
            pull_request_milestone_reconciliations,
            strict=True,
        ):
            validate_reconciliation_delivery_binding(event, result)
            resource = result.get("resource") if isinstance(result, dict) else None
            canonical_state = result.get("canonical_state") if isinstance(result, dict) else None
            milestone_projection = (
                canonical_state.get("milestone") if isinstance(canonical_state, dict) else None
            )
            if (
                status != 0
                or (not batch_pull_request_events and result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION)
                or result.get("verdict") != "current_state_read"
                or (
                    not batch_pull_request_events
                    and result.get("upstream_read_attempts") != 1
                )
                or not isinstance(resource, dict)
                or resource.get("kind") != "pull_request"
                or resource.get("number") != pull_request_number
                or resource.get("owner") != OWNER
                or resource.get("repository") != REPOSITORY
                or not is_current_unassigned_milestone_projection(milestone_projection)
            ):
                raise RuntimeError(
                    "PR milestone event did not reconcile to the current unassigned Forgejo API state"
                )
        expected_current_issue_state = "open" if reopen_issue else "closed"
        for event, status, result in (
            (open_event, open_status, open_reconciliation),
            (closed_event, closed_status, closed_reconciliation),
            *(
                [(issue_reopened_event, issue_reopened_status, issue_reopened_reconciliation)]
                if issue_reopened_event is not None
                else []
            ),
        ):
            validate_reconciliation_delivery_binding(event, result)
            if (
                status != 0
                or result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION
                or result.get("verdict") != "current_state_read"
                or result.get("upstream_read_attempts") != 1
                or result.get("resource", {}).get("number") != issue_number
                or result.get("resource", {}).get("owner") != OWNER
                or result.get("resource", {}).get("repository") != REPOSITORY
                or result.get("canonical_state", {}).get("state") != expected_current_issue_state
            ):
                raise RuntimeError("reconciliation did not return current Forgejo issue state")
        for event, status, result in zip(
            issue_label_events,
            issue_label_event_statuses,
            issue_label_reconciliations,
            strict=True,
        ):
            validate_reconciliation_delivery_binding(event, result)
            resource = result.get("resource") if isinstance(result, dict) else None
            canonical_state = result.get("canonical_state") if isinstance(result, dict) else None
            labels_projection = (
                canonical_state.get("labels") if isinstance(canonical_state, dict) else None
            )
            if (
                status != 0
                or result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION
                or result.get("verdict") != "current_state_read"
                or result.get("upstream_read_attempts") != 1
                or not isinstance(resource, dict)
                or resource.get("kind") != "issue"
                or resource.get("number") != issue_number
                or resource.get("owner") != OWNER
                or resource.get("repository") != REPOSITORY
                or not isinstance(canonical_state, dict)
                or not is_current_empty_label_projection(labels_projection)
            ):
                raise RuntimeError(
                    "issue label event did not reconcile to the current empty Forgejo API label set"
                )
        for event, status, result in zip(
            issue_assignee_events,
            issue_assignee_event_statuses,
            issue_assignee_reconciliations,
            strict=True,
        ):
            validate_reconciliation_delivery_binding(event, result)
            resource = result.get("resource") if isinstance(result, dict) else None
            canonical_state = result.get("canonical_state") if isinstance(result, dict) else None
            assignees_projection = (
                canonical_state.get("assignees") if isinstance(canonical_state, dict) else None
            )
            if (
                status != 0
                or result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION
                or result.get("verdict") != "current_state_read"
                or result.get("upstream_read_attempts") != 1
                or not isinstance(resource, dict)
                or resource.get("kind") != "issue"
                or resource.get("number") != issue_number
                or resource.get("owner") != OWNER
                or resource.get("repository") != REPOSITORY
                or not isinstance(canonical_state, dict)
                or (
                    issue_assignee_clear_api_verdict == "known_empty"
                    and not is_current_empty_assignee_projection(assignees_projection)
                )
                or (
                    issue_assignee_clear_api_verdict == "unknown_null_field"
                    and assignees_projection is not None
                )
            ):
                raise RuntimeError(
                    "issue assignment event did not reconcile to the current Forgejo API assignee certainty"
                )
        for event, status, result in zip(
            issue_milestone_events,
            issue_milestone_event_statuses,
            issue_milestone_reconciliations,
            strict=True,
        ):
            validate_reconciliation_delivery_binding(event, result)
            resource = result.get("resource") if isinstance(result, dict) else None
            canonical_state = result.get("canonical_state") if isinstance(result, dict) else None
            milestone_projection = (
                canonical_state.get("milestone") if isinstance(canonical_state, dict) else None
            )
            if (
                status != 0
                or result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION
                or result.get("verdict") != "current_state_read"
                or result.get("upstream_read_attempts") != 1
                or not isinstance(resource, dict)
                or resource.get("kind") != "issue"
                or resource.get("number") != issue_number
                or resource.get("owner") != OWNER
                or resource.get("repository") != REPOSITORY
                or not is_current_unassigned_milestone_projection(milestone_projection)
            ):
                raise RuntimeError(
                    "issue milestone event did not reconcile to the current unassigned Forgejo API state"
                )
        for event, status, result in zip(
            edited_events, edited_statuses, edited_reconciliations, strict=True
        ):
            validate_reconciliation_delivery_binding(event, result)
            if (
                status != 0
                or result.get("schema_version") != RECONCILE_SINGLE_SCHEMA_VERSION
                or result.get("verdict") != "current_state_read"
                or result.get("upstream_read_attempts") != 1
                or result.get("resource", {}).get("number") != issue_number
                or result.get("resource", {}).get("owner") != OWNER
                or result.get("resource", {}).get("repository") != REPOSITORY
                or result.get("canonical_state", {}).get("state") != expected_current_issue_state
            ):
                raise RuntimeError("reconciliation did not return current Forgejo state for an edited issue")
        for result in [
            open_reconciliation,
            closed_reconciliation,
            *edited_reconciliations,
            *(
                [issue_reopened_reconciliation]
                if issue_reopened_reconciliation is not None
                else []
            ),
        ]:
            validate_work_item_content_projection(
                result,
                expected_title=edited_title,
                expected_body=edited_body,
                expected_source="current_forgejo_issue_api",
            )
        if initial_issue_event_content is None or (
            initial_issue_event_content["title"] == edited_title
            and initial_issue_event_content["body"] == edited_body
        ):
            raise RuntimeError("live issue content test did not create stale webhook context")
        helpers.write_safe_json(
            evidence_dir / "work-item-content-readback.json",
            {
                "source_event": "Forgejo-emitted issues/opened event from the receiver journal",
                "initial_webhook_content": initial_issue_event_content,
                "reconciled_delivery_id": open_event["delivery_id"],
                "reconciled_payload_state": open_event["payload_state"],
                "current_api_content": {
                    "title": edited_title,
                    "body": edited_body,
                    "source": "current_forgejo_issue_api",
                    "trust": "untrusted_user_content",
                },
                "old_delivery_reconciled_after_edit": True,
                "initial_and_current_content_differ": True,
            },
            tokens,
        )
        if pull_request_open_reconciliation is not None:
            validate_work_item_content_projection(
                pull_request_open_reconciliation,
                expected_title=(
                    "Exercise emitted pull-request webhooks (edited)"
                    if edit_pull_request_title
                    else "Exercise emitted pull-request webhooks"
                ),
                expected_body="Verify that the live receiver journals and reconciles PR transitions.",
                expected_source="current_forgejo_pull_request_api",
                batch_item=batch_pull_request_events,
            )
        if (
            open_event["payload_state"] != "open"
            or closed_event["payload_state"] != "closed"
            or any(event["payload_state"] != "closed" for event in edited_events)
            or (
                issue_reopened_event is not None
                and issue_reopened_event["payload_state"] != "open"
            )
        ):
            raise RuntimeError("Forgejo's emitted payloads did not represent the expected state transitions")
        final_pull_request, _ = owner_api.request("GET", pull_request_path)
        final_head_sha = final_pull_request.get("head", {}).get("sha")
        final_base_sha = final_pull_request.get("base", {}).get("sha")
        final_merge_commit_sha = final_pull_request.get("merge_commit_sha")
        expected_final_merge_sha = (
            merge_response.get("merge_commit_sha") if merge_response else None
        )
        if (
            final_pull_request.get("state") != "closed"
            or final_pull_request.get("merged") is not args.merge_pull_request
            or final_pull_request.get("title")
            != (
                "Exercise emitted pull-request webhooks (edited)"
                if edit_pull_request_title
                else "Exercise emitted pull-request webhooks"
            )
            or final_head_sha != candidate_head_sha
            or (not args.merge_pull_request and final_base_sha != opened_base_sha)
            or (
                change_pull_request_labels
                and normalize_live_pull_request_labels(final_pull_request.get("labels"))
                != []
            )
            or (
                change_pull_request_milestones
                and (
                    "milestone" not in final_pull_request
                    or normalize_live_issue_milestone(final_pull_request["milestone"])
                    is not None
                )
            )
            or (
                args.merge_pull_request
                and (
                    final_merge_commit_sha != expected_final_merge_sha
                    or pull_request_closed_event.get("merge_commit_sha") != expected_final_merge_sha
                )
            )
        ):
            raise RuntimeError("Forgejo API readback did not confirm the expected closed PR state")
        if change_pull_request_assignees:
            final_pull_request_assignees = normalize_live_issue_assignees(
                final_pull_request.get("assignees")
            )
            if final_pull_request_assignees not in (None, []):
                raise RuntimeError("Forgejo final PR readback still reports assignees after the clear")
            if (
                pull_request_assignee_clear_api_verdict == "known_empty"
                and final_pull_request_assignees != []
            ) or (
                pull_request_assignee_clear_api_verdict == "unknown_null_field"
                and final_pull_request_assignees is not None
            ):
                raise RuntimeError("Forgejo final PR assignee readback changed its certainty")
        if args.merge_pull_request:
            merged_main_file, _ = owner_api.request(
                "GET",
                f"/repos/{OWNER}/{REPOSITORY}/contents/webhook-e2e.txt?ref=main",
            )
            if (
                not isinstance(merged_main_file, dict)
                or base64.b64decode(merged_main_file.get("content", ""), validate=True)
                != synchronized_content
            ):
                raise RuntimeError("the merged PR file is absent or differs on Forgejo main")
            merged_main_branch, _ = owner_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main"
            )
            if merged_main_branch.get("commit", {}).get("id") != expected_final_merge_sha:
                raise RuntimeError("Forgejo main does not point to the reported PR merge commit")
        pull_request_reconciliations = [
            (pull_request_open_event, pull_request_open_status, pull_request_open_reconciliation),
        ]
        if pull_request_sync_event is not None:
            pull_request_reconciliations.append(
                (
                    pull_request_sync_event,
                    pull_request_sync_status,
                    pull_request_sync_reconciliation,
                )
            )
        pull_request_reconciliations.extend(
            [
                *(
                    [
                        (
                            pull_request_comment_event,
                            pull_request_comment_status,
                            pull_request_comment_reconciliation,
                        )
                    ]
                    if not revoke_pull_request_comment_reader
                    else []
                ),
                *(
                    [
                        (
                            pull_request_review_comment_event,
                            pull_request_review_comment_status,
                            pull_request_review_comment_reconciliation,
                        )
                    ]
                    if pull_request_review_comment_event is not None
                    else []
                ),
                *(
                    [
                        (
                            pull_request_rejected_review_event,
                            pull_request_rejected_review_status,
                            pull_request_rejected_review_reconciliation,
                        )
                    ]
                    if pull_request_rejected_review_event is not None
                    else []
                ),
                *(
                    [
                        (
                            pull_request_approved_review_event,
                            pull_request_approved_review_status,
                            pull_request_approved_review_reconciliation,
                        )
                    ]
                    if pull_request_approved_review_event is not None
                    else []
                ),
                *(
                    [
                        (
                            pull_request_review_request_event,
                            pull_request_review_request_status,
                            pull_request_review_request_reconciliation,
                        )
                    ]
                    if pull_request_review_request_event is not None
                    else []
                ),
                *(
                    [
                        (
                            pull_request_review_request_removed_event,
                            pull_request_review_request_removed_status,
                            pull_request_review_request_removed_reconciliation,
                        )
                    ]
                    if pull_request_review_request_removed_event is not None
                    else []
                ),
                *(
                    [
                        (
                            pull_request_title_edit_event,
                            pull_request_title_edit_status,
                            pull_request_title_edit_reconciliation,
                        )
                    ]
                    if pull_request_title_edit_event is not None
                    else []
                ),
                *(
                    [
                        (
                            pull_request_pre_reopen_closed_event,
                            pull_request_pre_reopen_closed_status,
                            pull_request_pre_reopen_closed_reconciliation,
                        )
                    ]
                    if pull_request_pre_reopen_closed_event is not None
                    else []
                ),
                *(
                    [
                        (
                            pull_request_reopened_event,
                            pull_request_reopened_status,
                            pull_request_reopened_reconciliation,
                        )
                    ]
                    if pull_request_reopened_event is not None
                    else []
                ),
                (
                    pull_request_closed_event,
                    pull_request_closed_status,
                    pull_request_closed_reconciliation,
                ),
            ]
        )
        for event, status, result in pull_request_reconciliations:
            assert event is not None and status is not None and result is not None
            validate_reconciliation_delivery_binding(event, result)
            canonical = result.get("canonical_state", {})
            if (
                status != 0
                or result.get("verdict") != "current_state_read"
                or (
                    pull_request_batch_reconciliation is None
                    and result.get("upstream_read_attempts")
                    != (
                        3
                        if event is pull_request_comment_event
                        and event.get("action") == "deleted"
                        and pull_request_comment_delete_http_status == 404
                        else 2
                        if event is pull_request_comment_event
                        else 1
                    )
                )
                or result.get("resource", {}).get("kind") != "pull_request"
                or result.get("resource", {}).get("number") != pull_request_number
                or result.get("resource", {}).get("owner") != OWNER
                or result.get("resource", {}).get("repository") != REPOSITORY
                or canonical.get("state") != "closed"
                or canonical.get("merged") is not args.merge_pull_request
                or canonical.get("head_sha") != final_head_sha
                or canonical.get("base_sha") != final_base_sha
                or canonical.get("merge_commit_sha") != expected_final_merge_sha
            ):
                raise RuntimeError("reconciliation did not return the current canonical Forgejo PR state")
            if event is pull_request_comment_event:
                assert pull_request_comment_live_readback is not None
                validate_pull_request_comment_reconciliation(
                    status,
                    event,
                    result,
                    (
                        None
                        if event.get("action") == "deleted"
                        else pull_request_comment_live_readback
                    ),
                    owner=OWNER,
                    repository=REPOSITORY,
                    pull_request_number=pull_request_number,
                    live_comment_http_status=(
                        pull_request_comment_delete_http_status
                        if event.get("action") == "deleted"
                        else 200
                    ),
                    batch_item=pull_request_batch_reconciliation is not None,
                )

        if revoke_pull_request_comment_reader:
            assert (
                owner_api is not None
                and reader_api is not None
                and pull_request_number is not None
                and pull_request_comment_created_event is not None
                and pull_request_comment_deleted_event is not None
            )
            stage = "revoke_comment_reader_and_confirm_parent_visibility"
            owner_api.request(
                "DELETE",
                f"/repos/{OWNER}/{REPOSITORY}/collaborators/{quote(READER, safe='')}",
            )
            revoke_http_status = owner_api.last_status
            if revoke_http_status != 204:
                raise RuntimeError("Forgejo did not confirm revoking the comment reader")

            reader_account_after_revoke, _ = reader_api.request("GET", "/user")
            reader_identity_http_status = reader_api.last_status
            if (
                reader_identity_http_status != 200
                or not isinstance(reader_account_after_revoke, dict)
                or reader_account_after_revoke.get("login") != READER
                or reader_account_after_revoke.get("is_admin") is not False
            ):
                raise RuntimeError("the revoked reader token did not remain valid for identity readback")

            reader_api.request(
                "GET",
                f"/repos/{OWNER}/{REPOSITORY}/pulls/{pull_request_number}",
                expected_http_error_statuses=(404,),
            )
            reader_parent_http_status = reader_api.last_status
            if reader_parent_http_status != 404:
                raise RuntimeError("revoked reader could still read the private PR parent")

            owner_parent_after_revoke, _ = owner_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/pulls/{pull_request_number}"
            )
            owner_parent_http_status = owner_api.last_status
            if (
                owner_parent_http_status != 200
                or not isinstance(owner_parent_after_revoke, dict)
                or owner_parent_after_revoke.get("number") != pull_request_number
                or owner_parent_after_revoke.get("html_url")
                != f"{ORIGIN}/{OWNER}/{REPOSITORY}/pulls/{pull_request_number}"
            ):
                raise RuntimeError("owner could not confirm the PR still exists after reader revocation")
            pull_request_comment_parent_visibility = {
                "permission_change": "owner_revoked_read_only_collaborator",
                "revoke_http_status": revoke_http_status,
                "reader_identity_http_status_after_revoke": reader_identity_http_status,
                "reader_login_after_revoke": reader_account_after_revoke["login"],
                "reader_parent_http_status": reader_parent_http_status,
                "owner_parent_http_status": owner_parent_http_status,
                "owner_parent_number": owner_parent_after_revoke["number"],
                "owner_parent_html_url": owner_parent_after_revoke["html_url"],
            }

            stage = "reconcile_comments_with_unreadable_parent"
            for comment_event in (
                pull_request_comment_created_event,
                pull_request_comment_deleted_event,
            ):
                comment_status, comment_reconciliation = helpers.run_reconcile(
                    reconcile_bin,
                    state,
                    journal_path,
                    comment_event["delivery_id"],
                    reader_token,
                )
                validate_pull_request_comment_parent_unreadable(
                    comment_status,
                    comment_event,
                    comment_reconciliation,
                    owner=OWNER,
                    repository=REPOSITORY,
                    pull_request_number=pull_request_number,
                )
                if comment_event["action"] == "created":
                    pull_request_comment_created_status = comment_status
                    pull_request_comment_created_reconciliation = comment_reconciliation
                else:
                    pull_request_comment_status = comment_status
                    pull_request_comment_reconciliation = comment_reconciliation

        stage = "refuse_invalid_reader_credential"
        invalid_token = "invalid-" + secrets.token_urlsafe(24)
        tokens.append(invalid_token)
        invalid_status, invalid_result = helpers.run_reconcile(
            reconcile_bin, state, journal_path, open_event["delivery_id"], invalid_token
        )
        validate_reconciliation_delivery_binding(open_event, invalid_result)
        if (
            invalid_status == 0
            or invalid_result.get("verdict") != "inconclusive"
            or invalid_result.get("error", {}).get("code") != "upstream_unauthorized"
            or invalid_result.get("error", {}).get("retryable") is not False
            or invalid_result.get("upstream_read_attempts") != 1
            or "canonical_state" in invalid_result
        ):
            raise RuntimeError("invalid Forgejo credential did not remain inconclusive")
        post_issue, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
        )
        expected_final_issue_state = "open" if reopen_issue else "closed"
        if post_issue.get("state") != expected_final_issue_state:
            raise RuntimeError("Forgejo API readback did not confirm the expected final issue state")
        if post_issue.get("title") != edited_title or post_issue.get("body") != edited_body:
            raise RuntimeError("Forgejo API readback did not confirm the edited issue content")
        if change_issue_labels and normalize_live_issue_labels(post_issue.get("labels")):
            raise RuntimeError("Forgejo final issue readback still reports labels after the clear")
        if change_issue_assignees:
            final_issue_assignees = normalize_current_issue_api_assignees(post_issue)
            if final_issue_assignees not in (None, []):
                raise RuntimeError("Forgejo final issue readback still reports assignees after the clear")
            if (
                issue_assignee_clear_api_verdict == "known_empty"
                and final_issue_assignees != []
            ) or (
                issue_assignee_clear_api_verdict == "unknown_null_field"
                and final_issue_assignees is not None
            ):
                raise RuntimeError("Forgejo final assignee readback changed its certainty")
        if change_issue_milestones:
            if "milestone" not in post_issue:
                raise RuntimeError("Forgejo final issue API omitted its milestone field")
            if normalize_live_issue_milestone(post_issue["milestone"]) is not None:
                raise RuntimeError("Forgejo final issue readback still reports a milestone")

        stage = "disable_test_webhook"
        owner_api.request(
            "PATCH",
            f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
            hook_deactivation_patch(),
        )
        hook_api_readback, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}"
        )
        hook_readback = hook_readback_record(hook_id, hook_api_readback, expected_hook_url)
        hook_disabled = hook_readback["deactivation_readback_confirmed"] is True
        if not hook_disabled:
            raise RuntimeError("Forgejo hook readback did not confirm deactivation")
        if not hook_readback["configuration_matches_expected"]:
            raise RuntimeError("Forgejo hook readback differs from the created hook configuration")

        source_head = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "HEAD"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=10,
            check=True,
        ).stdout.strip()
        safe_result = {
            "schema_version": 1,
            "verdict": "passed",
            "forgejo": {
                "version": version.stdout.strip(),
                "binary_sha256": helpers.sha256_file(forgejo),
                "listener": "127.0.0.1:3130",
                "test_only_webhook_allowlist": "127.0.0.1",
            },
            "repository": {
                "full_name": f"{OWNER}/{REPOSITORY}",
                "private": True,
                "api_reader": {
                    "login": READER,
                    "is_admin": reader_account.get("is_admin"),
                    "repo_permission_admin": permissions.get("admin"),
                    "repo_permission_push": permissions.get("push"),
                    "repo_permission_pull": permissions.get("pull"),
                    "requested_token_scopes": reader_scopes.split(","),
                },
                "pull_request_author": (
                    {
                        "login": contributor_account.get("login"),
                        "is_admin": contributor_account.get("is_admin"),
                        "repo_permission_admin": contributor_permissions.get("admin"),
                        "repo_permission_push": contributor_permissions.get("push"),
                        "repo_permission_pull": contributor_permissions.get("pull"),
                        "requested_token_scopes": contributor_token_scopes.split(","),
                        "fork_repository": (
                            {
                                "full_name": contributor_fork_repository.get("full_name"),
                                "private": contributor_fork_repository.get("private"),
                                "fork": contributor_fork_repository.get("fork"),
                                "parent_full_name": contributor_fork_repository.get("parent", {}).get("full_name"),
                            }
                            if contributor_fork_repository is not None
                            else None
                        ),
                    }
                    if contributor_account is not None and contributor_permissions is not None
                    else None
                ),
            },
            "webhook": {
                "emitter": "Forgejo v17 actual webhook task",
                **hook_readback,
                "delivery_events": all_events,
                "forgejo_task_readback": hook_tasks,
                "duplicate_replay": duplicate_replay,
                "secret_recorded": False,
            },
            "issue": {
                "number": issue_number,
                "state_after_run": post_issue.get("state"),
                "old_open_event_payload_state": open_event["payload_state"],
                "edited_events": edited_events,
                "reopen_requested": reopen_issue,
                "reopen_http_status": issue_reopen_http_status,
                "reopened_api_readback_state": (
                    issue_reopened_api_readback.get("state")
                    if issue_reopened_api_readback is not None
                    else None
                ),
                "reopened_event": issue_reopened_event,
                "reopened_event_reconciliation": issue_reopened_reconciliation,
                "reconciled_current_state": open_reconciliation.get("canonical_state", {}).get("state"),
                "edited_event_reconciliations": edited_reconciliations,
                "label_change_requested": change_issue_labels,
                "label_id": issue_label_id,
                "label_name": issue_label_name,
                "label_create_http_status": issue_label_create_status,
                "label_add_http_status": issue_label_add_status,
                "label_clear_http_status": issue_label_clear_status,
                "label_events": [
                    {
                        "action": event["action"],
                        "delivery_id": event["delivery_id"],
                        "sequence": event["sequence"],
                        "payload_labels": payload_labels,
                        "api_labels_after_transition": api_labels,
                        "reconcile_exit_status": status,
                        "reconciliation": reconciliation,
                    }
                    for event, payload_labels, api_labels, status, reconciliation in zip(
                        issue_label_events,
                        issue_label_event_payloads,
                        issue_label_api_readbacks,
                        issue_label_event_statuses,
                        issue_label_reconciliations,
                        strict=True,
                    )
                ],
                "assignee_change_requested": change_issue_assignees,
                "assignee_add_http_status": issue_assignee_add_status,
                "assignee_clear_http_status": issue_assignee_clear_status,
                "assignee_clear_api_readback_verdict": issue_assignee_clear_api_verdict,
                "assignee_events": [
                    {
                        "action": event["action"],
                        "delivery_id": event["delivery_id"],
                        "sequence": event["sequence"],
                        "payload_assignees": payload_assignees,
                        "api_assignees_after_transition": api_assignees,
                        "reconcile_exit_status": status,
                        "reconciliation": reconciliation,
                    }
                    for event, payload_assignees, api_assignees, status, reconciliation in zip(
                        issue_assignee_events,
                        issue_assignee_event_payloads,
                        issue_assignee_api_readbacks,
                        issue_assignee_event_statuses,
                        issue_assignee_reconciliations,
                        strict=True,
                    )
                ],
                "milestone_change_requested": change_issue_milestones,
                "milestone_id": issue_milestone_id,
                "milestone_title": issue_milestone_title,
                "milestone_create_http_status": issue_milestone_create_status,
                "milestone_assign_http_status": issue_milestone_assign_status,
                "milestone_clear_http_status": issue_milestone_clear_status,
                "milestone_clear_api_readback_verdict": issue_milestone_clear_api_verdict,
                "milestone_events": [
                    {
                        "action": event["action"],
                        "delivery_id": event["delivery_id"],
                        "sequence": event["sequence"],
                        "payload_milestone": payload_milestone,
                        "api_milestone_after_transition": api_milestone,
                        "reconcile_exit_status": status,
                        "reconciliation": reconciliation,
                    }
                    for event, payload_milestone, api_milestone, status, reconciliation in zip(
                        issue_milestone_events,
                        issue_milestone_event_payloads,
                        issue_milestone_api_readbacks,
                        issue_milestone_event_statuses,
                        issue_milestone_reconciliations,
                        strict=True,
                    )
                ],
                "ordinary_comment_requested": ordinary_issue_comment,
                "ordinary_comment_actions_requested": ordinary_issue_comment_actions,
                "ordinary_comment_id": ordinary_issue_comment_id,
                "ordinary_comment_delete_readback": ordinary_issue_comment_delete_readback,
                "ordinary_comment_event": ordinary_issue_comment_event,
                "ordinary_comment_reconciliation": ordinary_issue_comment_reconciliation,
                "ordinary_comment_events": ordinary_issue_comment_events,
                "ordinary_comment_reconcile_exit_statuses": ordinary_issue_comment_statuses,
                "ordinary_comment_reconciliations": ordinary_issue_comment_reconciliations,
            },
            "pull_request": {
                "number": pull_request_number,
                "from_fork": fork_pull_request,
                "base_repository_full_name": pull_request_base_repository_full_name,
                "head_repository_full_name": pull_request_head_repository_full_name,
                "head_sha": final_head_sha,
                "opened_head_sha": opened_head_sha,
                "synchronized": synchronize_pull_request,
                "reopen_scenario": reopen_pull_request,
                "title_edit_requested": edit_pull_request_title,
                "title_edit_http_status": pull_request_title_edit_http_status,
                "title_after_run": final_pull_request.get("title"),
                "synchronized_head_sha": candidate_head_sha if synchronize_pull_request else None,
                "base_sha": final_base_sha,
                "state_after_run": final_pull_request.get("state"),
                "merged_after_run": final_pull_request.get("merged"),
                "merge_commit_sha": final_merge_commit_sha,
                "opened_event_payload_state": pull_request_open_event["payload_state"],
                "synchronized_event_delivery_id": (
                    pull_request_sync_event["delivery_id"]
                    if pull_request_sync_event is not None
                    else None
                ),
                "synchronized_event_action": (
                    pull_request_sync_event["action"]
                    if pull_request_sync_event is not None
                    else None
                ),
                "synchronized_event_head_sha": (
                    pull_request_sync_event["head_sha"]
                    if pull_request_sync_event is not None
                    else None
                ),
                "label_change_requested": change_pull_request_labels,
                "label_id": pull_request_label_id,
                "label_name": pull_request_label_name,
                "label_create_http_status": pull_request_label_create_status,
                "label_add_http_status": pull_request_label_add_status,
                "label_clear_http_status": pull_request_label_clear_status,
                "label_events": [
                    {
                        "action": event["action"],
                        "delivery_id": event["delivery_id"],
                        "sequence": event["sequence"],
                        "payload_labels": payload_labels,
                        "api_labels_after_transition": api_labels,
                    }
                    for event, payload_labels, api_labels in zip(
                        pull_request_label_events,
                        pull_request_label_event_payloads,
                        pull_request_label_api_readbacks,
                        strict=True,
                    )
                ],
                "assignee_change_requested": change_pull_request_assignees,
                "assignee_add_http_status": pull_request_assignee_add_status,
                "assignee_clear_http_status": pull_request_assignee_clear_status,
                "assignee_clear_api_readback_verdict": pull_request_assignee_clear_api_verdict,
                "assignee_events": [
                    {
                        "action": event["action"],
                        "delivery_id": event["delivery_id"],
                        "sequence": event["sequence"],
                        "payload_assignees": payload_assignees,
                        "api_assignees_after_transition": api_assignees,
                        "reconcile_exit_status": status,
                        "reconciliation": reconciliation,
                    }
                    for event, payload_assignees, api_assignees, status, reconciliation in zip(
                        pull_request_assignee_events,
                        pull_request_assignee_event_payloads,
                        pull_request_assignee_api_readbacks,
                        pull_request_assignee_event_statuses,
                        pull_request_assignee_reconciliations,
                        strict=True,
                    )
                ],
                "milestone_change_requested": change_pull_request_milestones,
                "milestone_id": pull_request_milestone_id,
                "milestone_title": pull_request_milestone_title,
                "milestone_create_http_status": pull_request_milestone_create_status,
                "milestone_assign_http_status": pull_request_milestone_assign_status,
                "milestone_clear_http_status": pull_request_milestone_clear_status,
                "milestone_clear_api_readback_verdict": pull_request_milestone_clear_api_verdict,
                "milestone_events": [
                    {
                        "action": event["action"],
                        "delivery_id": event["delivery_id"],
                        "sequence": event["sequence"],
                        "payload_milestone": payload_milestone,
                        "api_milestone_after_transition": api_milestone,
                        "reconcile_exit_status": status,
                        "reconciliation": reconciliation,
                    }
                    for event, payload_milestone, api_milestone, status, reconciliation in zip(
                        pull_request_milestone_events,
                        pull_request_milestone_event_payloads,
                        pull_request_milestone_api_readbacks,
                        pull_request_milestone_event_statuses,
                        pull_request_milestone_reconciliations,
                        strict=True,
                    )
                ],
                "conversation_comment_id": pull_request_comment_id,
                "conversation_comment_parent_visibility_after_reader_revocation": (
                    pull_request_comment_parent_visibility
                ),
                "conversation_comment_action": pull_request_comment_event["action"],
                "conversation_comment_live_api_readback": (
                    {
                        field: pull_request_comment_live_readback.get(field)
                        for field in ("id", "body", "updated_at", "html_url", "pull_request_url")
                    }
                    if pull_request_comment_live_readback is not None
                    else None
                ),
                "conversation_comment_delete_api_readback": pull_request_comment_delete_readback,
                "conversation_comment_created_delivery_id": (
                    pull_request_comment_created_event["delivery_id"]
                    if delete_pull_request_comment
                    and pull_request_comment_created_event is not None
                    else None
                ),
                "conversation_comment_stale_created_exit_status": (
                    pull_request_comment_created_status
                ),
                "conversation_comment_stale_created_reconciliation": (
                    pull_request_comment_created_reconciliation
                ),
                "conversation_comment_delivery_id": pull_request_comment_event["delivery_id"],
                "conversation_comment_payload_state": pull_request_comment_event["payload_state"],
                "inline_review_comment_requested": pull_request_review_comment,
                "inline_review_comment_delivery_id": (
                    pull_request_review_comment_event["delivery_id"]
                    if pull_request_review_comment_event is not None
                    else None
                ),
                "inline_review_comment_type": (
                    pull_request_review_comment_event["review_type"]
                    if pull_request_review_comment_event is not None
                    else None
                ),
                "inline_review_comment_commit_id": (
                    pull_request_review_comment_response.get("commit_id")
                    if pull_request_review_comment_response is not None
                    else None
                ),
                "requested_changes_review": request_changes_review,
                "approved_review_requested": approve_review,
                "approved_review_state": (
                    pull_request_approved_review_response.get("state")
                    if pull_request_approved_review_response is not None
                    else None
                ),
                "approved_review_http_status": pull_request_approved_review_http_status,
                "approved_review_commit_id": (
                    pull_request_approved_review_response.get("commit_id")
                    if pull_request_approved_review_response is not None
                    else None
                ),
                "approved_review_delivery_id": (
                    pull_request_approved_review_event["delivery_id"]
                    if pull_request_approved_review_event is not None
                    else None
                ),
                "approved_review_type": (
                    pull_request_approved_review_event["review_type"]
                    if pull_request_approved_review_event is not None
                    else None
                ),
                "review_request_requested": request_review,
                "review_request_http_status": review_request_http_status,
                "requested_reviewers_readback": requested_reviewers_readback,
                "review_request_delivery_id": (
                    pull_request_review_request_event["delivery_id"]
                    if pull_request_review_request_event is not None
                    else None
                ),
                "review_request_reviewer_login": (
                    pull_request_review_request_event["requested_reviewer_login"]
                    if pull_request_review_request_event is not None
                    else None
                ),
                "review_request_reviewer_id": (
                    pull_request_review_request_event["requested_reviewer_id"]
                    if pull_request_review_request_event is not None
                    else None
                ),
                "review_request_removed": remove_review_request,
                "review_request_removal_http_status": review_request_removal_http_status,
                "requested_reviewers_after_removal_readback": requested_reviewers_after_removal_readback,
                "review_request_removed_delivery_id": (
                    pull_request_review_request_removed_event["delivery_id"]
                    if pull_request_review_request_removed_event is not None
                    else None
                ),
                "review_request_removed_action": (
                    pull_request_review_request_removed_event["action"]
                    if pull_request_review_request_removed_event is not None
                    else None
                ),
                "title_edit_delivery_id": (
                    pull_request_title_edit_event["delivery_id"]
                    if pull_request_title_edit_event is not None
                    else None
                ),
                "title_edit_payload_title": (
                    pull_request_title_edit_event["payload_title"]
                    if pull_request_title_edit_event is not None
                    else None
                ),
                "title_edit_changed_from": (
                    pull_request_title_edit_event["changed_title_from"]
                    if pull_request_title_edit_event is not None
                    else None
                ),
                "pre_reopen_closed_delivery_id": (
                    pull_request_pre_reopen_closed_event["delivery_id"]
                    if pull_request_pre_reopen_closed_event is not None
                    else None
                ),
                "pre_reopen_closed_action": (
                    pull_request_pre_reopen_closed_event["action"]
                    if pull_request_pre_reopen_closed_event is not None
                    else None
                ),
                "reopened_delivery_id": (
                    pull_request_reopened_event["delivery_id"]
                    if pull_request_reopened_event is not None
                    else None
                ),
                "reopened_action": (
                    pull_request_reopened_event["action"]
                    if pull_request_reopened_event is not None
                    else None
                ),
                "reopened_payload_state": (
                    pull_request_reopened_event["payload_state"]
                    if pull_request_reopened_event is not None
                    else None
                ),
                "batch_reconciliation": batch_pull_request_events,
                "review_author": OWNER if (request_changes_review or approve_review) else None,
                "rejected_review_delivery_id": (
                    pull_request_rejected_review_event["delivery_id"]
                    if pull_request_rejected_review_event is not None
                    else None
                ),
                "rejected_review_type": (
                    pull_request_rejected_review_event["review_type"]
                    if pull_request_rejected_review_event is not None
                    else None
                ),
                "closed_event_payload_state": pull_request_closed_event["payload_state"],
                "closed_event_payload_merged": pull_request_closed_event["payload_merged"],
                "closed_event_merge_commit_sha": pull_request_closed_event.get("merge_commit_sha"),
                "reconciled_current_state": pull_request_closed_reconciliation.get(
                    "canonical_state", {}
                ).get("state"),
            },
            "reconciliation": {
                "old_open_event": open_reconciliation,
                "new_closed_event": closed_reconciliation,
                "edited_closed_events": edited_reconciliations,
                "old_open_pull_request_event": pull_request_open_reconciliation,
                "pull_request_synchronized_event": pull_request_sync_reconciliation,
                "pull_request_conversation_comment_stale_created": (
                    pull_request_comment_created_reconciliation
                ),
                "pull_request_conversation_comment": pull_request_comment_reconciliation,
                "pull_request_inline_review_comment": pull_request_review_comment_reconciliation,
                "pull_request_rejected_review": pull_request_rejected_review_reconciliation,
                "pull_request_approved_review": pull_request_approved_review_reconciliation,
                "pull_request_review_request": pull_request_review_request_reconciliation,
                "pull_request_review_request_removed": pull_request_review_request_removed_reconciliation,
                "pull_request_title_edit": pull_request_title_edit_reconciliation,
                "pull_request_label_events": pull_request_label_reconciliations,
                "pull_request_assignee_events": pull_request_assignee_reconciliations,
                "pull_request_milestone_events": pull_request_milestone_reconciliations,
                "pull_request_closed_before_reopen": pull_request_pre_reopen_closed_reconciliation,
                "pull_request_reopened": pull_request_reopened_reconciliation,
                "new_closed_pull_request_event": pull_request_closed_reconciliation,
                "pull_request_batch": pull_request_batch_reconciliation,
                "batch_network_observation": batch_network_observation,
            },
            "invalid_credential": {
                "exit_status": invalid_status,
                "result": invalid_result,
                "credential_material_recorded": False,
            },
            "source_head_commit": source_head,
            "limitations": [
                "The webhook allowlist is loopback-only and belongs to this disposable local instance.",
                "Forgejo, receiver and CLI run as the current host user; hostile candidate code is not executed.",
                "No LLM, AgentOS, RAG or memory service was used; no comparative productivity or cost result is measured.",
            ],
        }
        stage = "complete"
    except Exception as error:
        failure = type(error).__name__
        failure_message = str(error)
        for token in tokens:
            if token:
                failure_message = failure_message.replace(token, "[REDACTED]")
        if webhook_secret:
            failure_message = failure_message.replace(webhook_secret, "[REDACTED]")
        failure_message = failure_message[:500]
    finally:
        if hook_id is not None and owner_api is not None and not hook_disabled:
            try:
                owner_api.request(
                    "PATCH",
                    f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
                    hook_deactivation_patch(),
                )
                hook_api_readback, _ = owner_api.request(
                    "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}"
                )
                hook_readback = hook_readback_record(
                    hook_id, hook_api_readback, expected_hook_url
                )
                hook_disabled = hook_readback["deactivation_readback_confirmed"] is True
                if not hook_disabled:
                    cleanup_failure = "hook_deactivation_not_confirmed"
            except Exception as error:
                cleanup_failure = type(error).__name__
        receiver_stop = helpers.stop_process(receiver)
        receiver_stop_history.append(
            {
                "phase": "final_cleanup",
                "stop": receiver_stop,
                "listener_addresses_after_stop": (
                    helpers.listener_addresses_for_port(receiver_port)
                    if receiver is not None and receiver_port is not None
                    else []
                ),
                "listener_checked": receiver is not None and receiver_port is not None,
            }
        )
        server_stop = helpers.stop_process(server)
        receiver_log.close()
        server_log.close()

    assert receiver_stop is not None and server_stop is not None
    cleanup_readback = {
        "forgejo": {
            "expected_address": "127.0.0.1:3130",
            "stop": server_stop,
            "listener_addresses_after_stop": (
                helpers.listener_addresses_for_port(3130) if server is not None else []
            ),
            "listener_checked": server is not None,
        },
        "receiver": {
            "expected_address": f"127.0.0.1:{receiver_port}" if receiver_port is not None else None,
            "stop": receiver_stop,
            "stops": receiver_stop_history,
            "listener_addresses_after_stop": (
                helpers.listener_addresses_for_port(receiver_port)
                if receiver is not None and receiver_port is not None
                else []
            ),
            "listener_checked": receiver is not None and receiver_port is not None,
        },
        "hook_disabled_before_server_stop": hook_disabled,
        "cleanup_failure": cleanup_failure,
    }
    cleanup_readback["tracked_processes_exited"] = bool(
        server_stop["stopped"]
        and all(entry["stop"]["stopped"] for entry in receiver_stop_history)
    )
    cleanup_readback["expected_tcp_listeners_absent"] = (
        not cleanup_readback["forgejo"]["listener_addresses_after_stop"]
        and not cleanup_readback["receiver"]["listener_addresses_after_stop"]
        and all(not entry["listener_addresses_after_stop"] for entry in receiver_stop_history)
    )
    helpers.write_safe_json(evidence_dir / "port-cleanup-readback.json", cleanup_readback, tokens)
    helpers.write_safe_json(
        evidence_dir / "hook-readback.json",
        hook_readback_record(hook_id, hook_api_readback, expected_hook_url),
        tokens,
    )
    helpers.write_safe_json(evidence_dir / "forgejo-hook-tasks.json", hook_tasks, tokens)

    if failure is not None or cleanup_failure is not None:
        helpers.write_safe_json(
            evidence_dir / "failure.json",
            {
                "verdict": "failed",
                "stage": stage,
                "error_type": failure,
                "error_message": failure_message,
                "cleanup_failure": cleanup_failure,
            },
            tokens,
        )
        helpers.write_checksums(evidence_dir)
        raise RuntimeError(f"Forgejo emitted webhook E2E failed at {stage}") from None

    assert safe_result is not None
    assert build_provenance is not None
    assert open_event is not None and closed_event is not None and edited_events is not None
    assert len(issue_assignee_events) == issue_assignee_event_count
    assert len(issue_assignee_event_statuses) == issue_assignee_event_count
    assert len(issue_assignee_reconciliations) == issue_assignee_event_count
    assert (issue_assignee_add_status is not None) == change_issue_assignees
    assert (issue_assignee_clear_status is not None) == change_issue_assignees
    assert len(issue_milestone_events) == issue_milestone_event_count
    assert len(issue_milestone_event_statuses) == issue_milestone_event_count
    assert len(issue_milestone_reconciliations) == issue_milestone_event_count
    assert (issue_milestone_create_status is not None) == change_issue_milestones
    assert (issue_milestone_assign_status is not None) == change_issue_milestones
    assert (issue_milestone_clear_status is not None) == change_issue_milestones
    assert (issue_milestone_clear_api_verdict is not None) == change_issue_milestones
    pull_request_assignee_event_count = 2 * int(change_pull_request_assignees)
    assert len(pull_request_assignee_events) == pull_request_assignee_event_count
    assert len(pull_request_assignee_event_statuses) == pull_request_assignee_event_count
    assert len(pull_request_assignee_reconciliations) == pull_request_assignee_event_count
    assert (pull_request_assignee_add_status is not None) == change_pull_request_assignees
    assert (pull_request_assignee_clear_status is not None) == change_pull_request_assignees
    assert (pull_request_assignee_clear_api_verdict is not None) == change_pull_request_assignees
    pull_request_milestone_event_count = 2 * int(change_pull_request_milestones)
    assert len(pull_request_milestone_events) == pull_request_milestone_event_count
    assert len(pull_request_milestone_event_statuses) == pull_request_milestone_event_count
    assert len(pull_request_milestone_reconciliations) == pull_request_milestone_event_count
    assert (pull_request_milestone_create_status is not None) == change_pull_request_milestones
    assert (pull_request_milestone_assign_status is not None) == change_pull_request_milestones
    assert (pull_request_milestone_clear_status is not None) == change_pull_request_milestones
    assert (pull_request_milestone_clear_api_verdict is not None) == change_pull_request_milestones
    assert (issue_reopened_event is not None) == reopen_issue
    assert (issue_reopened_event is None) == (issue_reopened_status is None)
    assert (issue_reopened_event is None) == (issue_reopened_reconciliation is None)
    assert (ordinary_issue_comment_event is None) == (
        ordinary_issue_comment_reconciliation is None
    )
    assert (ordinary_issue_comment_event is None) == (ordinary_issue_comment_status is None)
    assert (ordinary_issue_comment_event is None) == (ordinary_issue_comment_id is None)
    assert len(ordinary_issue_comment_events) == ordinary_issue_comment_event_count
    assert len(ordinary_issue_comment_statuses) == ordinary_issue_comment_event_count
    assert len(ordinary_issue_comment_reconciliations) == ordinary_issue_comment_event_count
    assert ("deleted" in ordinary_issue_comment_actions) == (
        ordinary_issue_comment_delete_readback is not None
    )
    assert pull_request_open_event is not None
    assert (pull_request_sync_event is None) == (pull_request_sync_reconciliation is None)
    assert (pull_request_sync_event is None) == (pull_request_sync_status is None)
    assert (pull_request_title_edit_event is None) == (
        pull_request_title_edit_reconciliation is None
    )
    assert (pull_request_title_edit_event is None) == (pull_request_title_edit_status is None)
    assert (pull_request_title_edit_event is None) == (pull_request_title_edit_http_status is None)
    assert (pull_request_review_request_event is None) == (
        pull_request_review_request_reconciliation is None
    )
    assert (pull_request_review_request_event is None) == (
        pull_request_review_request_status is None
    )
    assert (pull_request_review_request_removed_event is None) == (
        pull_request_review_request_removed_reconciliation is None
    )
    assert (pull_request_review_request_removed_event is None) == (
        pull_request_review_request_removed_status is None
    )
    assert pull_request_comment_event is not None and pull_request_comment_id is not None
    assert (pull_request_comment_deleted_event is not None) == delete_pull_request_comment
    assert (pull_request_comment_delete_readback is not None) == delete_pull_request_comment
    assert (pull_request_comment_delete_http_status is not None) == delete_pull_request_comment
    assert (pull_request_comment_parent_visibility is not None) == revoke_pull_request_comment_reader
    assert (pull_request_comment_created_status is not None) == delete_pull_request_comment
    assert (pull_request_comment_created_reconciliation is not None) == delete_pull_request_comment
    assert (pull_request_review_comment_event is None) == (
        pull_request_review_comment_reconciliation is None
    )
    assert (pull_request_review_comment_event is None) == (
        pull_request_review_comment_status is None
    )
    assert (pull_request_review_comment_event is None) == (
        pull_request_review_comment_response is None
    )
    assert (pull_request_approved_review_event is None) == (
        pull_request_approved_review_reconciliation is None
    )
    assert (pull_request_approved_review_event is None) == (
        pull_request_approved_review_status is None
    )
    assert (pull_request_approved_review_event is None) == (
        pull_request_approved_review_response is None
    )
    assert (pull_request_approved_review_event is None) == (
        pull_request_approved_review_http_status is None
    )
    assert (pull_request_pre_reopen_closed_event is None) == (
        pull_request_pre_reopen_closed_reconciliation is None
    )
    assert (pull_request_pre_reopen_closed_event is None) == (
        pull_request_pre_reopen_closed_status is None
    )
    assert (pull_request_reopened_event is None) == (
        pull_request_reopened_reconciliation is None
    )
    assert (pull_request_reopened_event is None) == (pull_request_reopened_status is None)
    assert pull_request_closed_event is not None
    assert open_reconciliation is not None and closed_reconciliation is not None
    assert edited_reconciliations is not None
    assert pull_request_open_reconciliation is not None
    assert pull_request_comment_status is not None and pull_request_comment_reconciliation is not None
    assert pull_request_comment_live_readback is not None
    assert pull_request_closed_reconciliation is not None
    assert invalid_status is not None and invalid_result is not None
    if (
        not cleanup_readback["tracked_processes_exited"]
        or not cleanup_readback["expected_tcp_listeners_absent"]
        or not hook_disabled
    ):
        helpers.write_safe_json(
            evidence_dir / "failure.json",
            {"verdict": "failed", "stage": "cleanup_readback", "error_type": "CleanupNotConfirmed"},
            tokens,
        )
        helpers.write_checksums(evidence_dir)
        raise RuntimeError("local Forgejo, receiver or hook did not stop cleanly")

    safe_result["process_cleanup"] = cleanup_readback
    helpers.write_safe_json(evidence_dir / "result.json", safe_result, tokens)
    helpers.write_safe_json(
        evidence_dir / "webhook-events.json",
        [
            open_event,
            *issue_label_events,
            *issue_assignee_events,
            *issue_milestone_events,
            closed_event,
            *edited_events,
            *([issue_reopened_event] if issue_reopened_event is not None else []),
            pull_request_open_event,
            *([pull_request_sync_event] if pull_request_sync_event is not None else []),
            *(
                [pull_request_comment_created_event]
                if delete_pull_request_comment
                else []
            ),
            pull_request_comment_event,
            *(
                [pull_request_review_comment_event]
                if pull_request_review_comment_event is not None
                else []
            ),
            *(
                [pull_request_rejected_review_event]
                if pull_request_rejected_review_event is not None
                else []
            ),
            *(
                [pull_request_approved_review_event]
                if pull_request_approved_review_event is not None
                else []
            ),
            *(
                [pull_request_review_request_event]
                if pull_request_review_request_event is not None
                else []
            ),
            *(
                [pull_request_review_request_removed_event]
                if pull_request_review_request_removed_event is not None
                else []
            ),
            *(
                [pull_request_title_edit_event]
                if pull_request_title_edit_event is not None
                else []
            ),
            *(
                [pull_request_pre_reopen_closed_event]
                if pull_request_pre_reopen_closed_event is not None
                else []
            ),
            *(
                [pull_request_reopened_event]
                if pull_request_reopened_event is not None
                else []
            ),
            *pull_request_label_events,
            *pull_request_assignee_events,
            *pull_request_milestone_events,
            pull_request_closed_event,
            *ordinary_issue_comment_events,
        ],
        tokens,
    )
    helpers.write_safe_json(
        evidence_dir / "open-event-reconciliation.json", open_reconciliation, tokens
    )
    helpers.write_safe_json(
        evidence_dir / "closed-event-reconciliation.json", closed_reconciliation, tokens
    )
    helpers.write_safe_json(
        evidence_dir / "edited-closed-event-reconciliations.json", edited_reconciliations, tokens
    )
    if issue_reopened_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "issue-reopened-event-reconciliation.json",
            issue_reopened_reconciliation,
            tokens,
        )
    if ordinary_issue_comment_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "ordinary-issue-comment-event-reconciliation.json",
            ordinary_issue_comment_reconciliation,
            tokens,
        )
        helpers.write_safe_json(
            evidence_dir / "ordinary-issue-comment-event-reconciliations.json",
            ordinary_issue_comment_reconciliations,
            tokens,
        )
    if ordinary_issue_comment_delete_readback is not None:
        helpers.write_safe_json(
            evidence_dir / "ordinary-issue-comment-delete-readback.json",
            ordinary_issue_comment_delete_readback,
            tokens,
        )
    helpers.write_safe_json(
        evidence_dir / "pull-request-open-event-reconciliation.json",
        pull_request_open_reconciliation,
        tokens,
    )
    if pull_request_sync_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-sync-event-reconciliation.json",
            pull_request_sync_reconciliation,
            tokens,
        )
    helpers.write_safe_json(
        evidence_dir / "pull-request-conversation-comment-reconciliation.json",
        pull_request_comment_reconciliation,
        tokens,
    )
    helpers.write_safe_json(
        evidence_dir / "pull-request-conversation-comment-live-api-readback.json",
        {
            field: pull_request_comment_live_readback.get(field)
            for field in ("id", "body", "updated_at", "html_url", "pull_request_url")
        },
        tokens,
    )
    if pull_request_comment_delete_readback is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-conversation-comment-delete-api-readback.json",
            pull_request_comment_delete_readback,
            tokens,
        )
    if pull_request_comment_created_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir
            / "pull-request-conversation-comment-stale-created-reconciliation.json",
            pull_request_comment_created_reconciliation,
            tokens,
        )
    if pull_request_review_comment_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-inline-review-comment-reconciliation.json",
            pull_request_review_comment_reconciliation,
            tokens,
        )
    if pull_request_rejected_review_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-rejected-review-event-reconciliation.json",
            pull_request_rejected_review_reconciliation,
            tokens,
        )
    if pull_request_approved_review_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-approved-review-event-reconciliation.json",
            pull_request_approved_review_reconciliation,
            tokens,
        )
    if pull_request_review_request_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-review-request-event-reconciliation.json",
            pull_request_review_request_reconciliation,
            tokens,
        )
    if pull_request_review_request_removed_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-review-request-removed-event-reconciliation.json",
            pull_request_review_request_removed_reconciliation,
            tokens,
        )
    if pull_request_label_events:
        helpers.write_safe_json(
            evidence_dir / "pull-request-label-change-reconciliation.json",
            {
                "label_id": pull_request_label_id,
                "label_name": pull_request_label_name,
                "create_http_status": pull_request_label_create_status,
                "add_http_status": pull_request_label_add_status,
                "clear_http_status": pull_request_label_clear_status,
                "events": [
                    {
                        "action": event["action"],
                        "delivery_id": event["delivery_id"],
                        "sequence": event["sequence"],
                        "payload_labels": payload_labels,
                        "api_labels_after_transition": api_labels,
                    }
                    for event, payload_labels, api_labels in zip(
                        pull_request_label_events,
                        pull_request_label_event_payloads,
                        pull_request_label_api_readbacks,
                        strict=True,
                    )
                ],
                "reconciled_current_api_label_projections": [
                    result.get("canonical_state", {}).get("labels")
                    for result in pull_request_label_reconciliations
                ],
            },
            tokens,
        )
    if pull_request_milestone_events:
        helpers.write_safe_json(
            evidence_dir / "pull-request-milestone-change-reconciliation.json",
            {
                "milestone_id": pull_request_milestone_id,
                "milestone_title": pull_request_milestone_title,
                "create_http_status": pull_request_milestone_create_status,
                "assign_http_status": pull_request_milestone_assign_status,
                "clear_http_status": pull_request_milestone_clear_status,
                "clear_api_readback_verdict": pull_request_milestone_clear_api_verdict,
                "events": [
                    {
                        "action": event["action"],
                        "delivery_id": event["delivery_id"],
                        "sequence": event["sequence"],
                        "payload_milestone": payload_milestone,
                        "api_milestone_after_transition": api_milestone,
                        "reconcile_exit_status": status,
                        "reconciliation": reconciliation,
                    }
                    for event, payload_milestone, api_milestone, status, reconciliation in zip(
                        pull_request_milestone_events,
                        pull_request_milestone_event_payloads,
                        pull_request_milestone_api_readbacks,
                        pull_request_milestone_event_statuses,
                        pull_request_milestone_reconciliations,
                        strict=True,
                    )
                ],
                "reconciled_current_api_milestone_projections": [
                    result.get("canonical_state", {}).get("milestone")
                    for result in pull_request_milestone_reconciliations
                ],
            },
            tokens,
        )
    if pull_request_title_edit_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-title-edit-event-reconciliation.json",
            pull_request_title_edit_reconciliation,
            tokens,
        )
    if pull_request_pre_reopen_closed_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-closed-before-reopen-event-reconciliation.json",
            pull_request_pre_reopen_closed_reconciliation,
            tokens,
        )
    if pull_request_reopened_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-reopened-event-reconciliation.json",
            pull_request_reopened_reconciliation,
            tokens,
        )
    if pull_request_batch_reconciliation is not None:
        helpers.write_safe_json(
            evidence_dir / "pull-request-batch-reconciliation.json",
            pull_request_batch_reconciliation,
            tokens,
        )
    helpers.write_safe_json(
        evidence_dir / "pull-request-closed-event-reconciliation.json",
        pull_request_closed_reconciliation,
        tokens,
    )
    helpers.write_safe_json(
        evidence_dir / "invalid-credential-result.json",
        {"exit_status": invalid_status, "result": invalid_result},
        tokens,
    )
    helpers.write_safe_json(
        evidence_dir / "reader-role-readback.json",
        safe_result["repository"]["api_reader"],
        tokens,
    )
    source_files = sorted(
        {
            repo / "Cargo.toml",
            repo / "Cargo.lock",
            Path(__file__).resolve(),
            (SCRIPT_DIR / "run_forgejo_live_webhook_reconcile_e2e.py").resolve(),
            (SCRIPT_DIR / "run_forgejo_live_pr_merge_e2e.py").resolve(),
            (SCRIPT_DIR / "run_forgejo_mission_delivery_process_e2e.py").resolve(),
            (repo / "tests/test_forgejo_emitted_webhook_e2e.py").resolve(),
            *(repo / "src").rglob("*.rs"),
        }
    )
    provenance = {
        "head_commit": safe_result["source_head_commit"],
        "source_worktree_status": subprocess.run(
            [
                "git",
                "-C",
                str(repo),
                "status",
                "--porcelain",
                "--",
                "Cargo.toml",
                "Cargo.lock",
                "src",
                "scripts/run_forgejo_live_webhook_reconcile_e2e.py",
                "scripts/run_forgejo_live_webhook_reconcile_emitted.py",
                "scripts/run_forgejo_live_pr_merge_e2e.py",
                "scripts/run_forgejo_mission_delivery_process_e2e.py",
                "tests/test_forgejo_emitted_webhook_e2e.py",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=10,
            check=True,
        ).stdout.splitlines(),
        "source_sha256": {
            str(path.relative_to(repo)): helpers.sha256_file(path) for path in source_files
        },
        "forgejo_binary_sha256": safe_result["forgejo"]["binary_sha256"],
        "built_executable_sha256": {
            "forgejo-webhook-local": helpers.sha256_file(receiver_bin),
            "forgejo-reconcile-local": helpers.sha256_file(reconcile_bin),
        },
        "cargo_build": build_provenance,
        "port_cleanup_readback_sha256": helpers.sha256_file(
            evidence_dir / "port-cleanup-readback.json"
        ),
        "hook_readback_sha256": helpers.sha256_file(evidence_dir / "hook-readback.json"),
        "private_state_dir": str(state),
        "private_state_dir_is_not_in_evidence": True,
        "webhook_secret_recorded": False,
    }
    helpers.write_safe_json(evidence_dir / "provenance.json", provenance, tokens)
    helpers.write_checksums(evidence_dir)
    return safe_result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--repo-root", default=str(SCRIPT_DIR.parent))
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    parser.add_argument(
        "--ordinary-issue-comment",
        action="store_true",
        help="create one comment on the disposable ordinary issue and reconcile Forgejo's emitted issue_comment event",
    )
    parser.add_argument(
        "--edit-ordinary-issue-comment",
        action="store_true",
        help="edit the created ordinary issue comment and reconcile its native issue_comment/edited event",
    )
    parser.add_argument(
        "--delete-ordinary-issue-comment",
        action="store_true",
        help="delete the created ordinary issue comment and reconcile its native issue_comment/deleted event",
    )
    parser.add_argument(
        "--merge-pull-request",
        action="store_true",
        help="merge the disposable local PR instead of closing it unmerged and reconcile its native close event",
    )
    parser.add_argument(
        "--reopen-pull-request",
        action="store_true",
        help="close and reopen the disposable PR before its final close, reconciling both native transitions",
    )
    parser.add_argument(
        "--edit-pull-request-title",
        action="store_true",
        help="edit the disposable PR title and reconcile Forgejo's native pull_request/edited event",
    )
    parser.add_argument(
        "--change-pull-request-labels",
        action="store_true",
        help="add then clear a disposable PR label and reconcile Forgejo's native pull_request/label_updated and label_cleared events",
    )
    parser.add_argument(
        "--change-pull-request-assignees",
        action="store_true",
        help="assign then unassign a read-only local user and reconcile Forgejo's native pull_request/assigned and unassigned events",
    )
    parser.add_argument(
        "--change-pull-request-milestones",
        action="store_true",
        help="assign then clear a disposable PR milestone and reconcile Forgejo's native pull_request/milestoned and demilestoned events",
    )
    parser.add_argument(
        "--change-issue-labels",
        action="store_true",
        help="add then clear a disposable issue label and reconcile Forgejo's native issues/label_updated and label_cleared events",
    )
    parser.add_argument(
        "--change-issue-assignees",
        action="store_true",
        help="assign then unassign a read-only local user and reconcile Forgejo's native issues/assigned and unassigned events",
    )
    parser.add_argument(
        "--change-issue-milestones",
        action="store_true",
        help="create, assign and clear a disposable issue milestone and reconcile Forgejo's native issues/milestoned and demilestoned events",
    )
    parser.add_argument(
        "--reopen-issue",
        action="store_true",
        help="close, edit, and reopen the disposable issue, then reconcile its native issues/reopened event",
    )
    parser.add_argument(
        "--synchronize-pull-request",
        action="store_true",
        help="push a second candidate commit and reconcile Forgejo's native pull_request_sync event",
    )
    parser.add_argument(
        "--fork-pull-request",
        action="store_true",
        help="create a private contributor fork with read-only target access and reconcile its native pull-request event",
    )
    parser.add_argument(
        "--request-changes-review",
        action="store_true",
        help="create a separate non-admin PR author and reconcile Forgejo's native rejected-review event",
    )
    parser.add_argument(
        "--approve-review",
        action="store_true",
        help="create a separate non-admin PR author and reconcile Forgejo's native approved-review event",
    )
    parser.add_argument(
        "--request-review",
        action="store_true",
        help="request review from the scoped read-only collaborator and reconcile Forgejo's native review_requested event",
    )
    parser.add_argument(
        "--remove-review-request",
        action="store_true",
        help="request and then remove the scoped read-only collaborator's review request, reconciling both native events",
    )
    parser.add_argument(
        "--pull-request-review-comment",
        action="store_true",
        help="create an inline PR review comment and reconcile Forgejo's native review-comment event",
    )
    parser.add_argument(
        "--delete-pull-request-comment",
        action="store_true",
        help="delete the disposable PR conversation comment and verify stale-created versus deleted reconciliation",
    )
    parser.add_argument(
        "--revoke-pull-request-comment-reader-before-reconcile",
        action="store_true",
        help="after deleting the PR comment and reconciling other events, revoke reader access and require both comment events to stay inconclusive when the PR parent is unreadable",
    )
    parser.add_argument(
        "--batch-pull-request-events",
        action="store_true",
        help="reconcile all emitted pull-request deliveries in one CLI batch and one current-state read",
    )
    parser.add_argument(
        "--observe-batch-network",
        action="store_true",
        help="trace connect syscalls during the PR batch; require at least one and reject non-fixture destinations",
    )
    args = parser.parse_args()
    result = run(args)
    print(json.dumps(result, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
