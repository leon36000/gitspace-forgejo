#!/usr/bin/env python3
"""Inject a separately reviewed main-branch advance just before a PR merge.

This characterization harness reuses the disposable Forgejo v17 setup and
verifier from run_forgejo_live_pr_merge_e2e.py. It does not change that driver
or invoke a model. The target PR's final reads and proof check run normally;
the wrapper then merges a second, approved PR and immediately retries the
target merge using only its previously checked head SHA.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import run_forgejo_live_pr_merge_e2e as driver


class RaceCaptured(Exception):
    pass


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def classify_race_result(result: dict[str, Any]) -> str:
    target = result.get("target_pull_request_after_attempt", {})
    attempts = result.get("target_merge_attempts", [])
    readbacks = result.get("mergeability_readbacks_after_initial_refusal", [])
    review = result.get("concurrent_pull_request_review", {})
    competitor = result.get("concurrent_pull_request_after", {})
    advanced_base = result.get("main_sha_after_concurrent_merge_before_target_merge")

    approved_competitor = (
        review.get("state") == "APPROVED"
        and review.get("commit_id") == result.get("concurrent_pull_request_approved_head")
        and result.get("concurrent_pull_request_merge_http_status") == 200
        and competitor.get("state") == "closed"
        and competitor.get("merged") is True
        and competitor.get("head_sha")
        == result.get("concurrent_pull_request_approved_head")
        and advanced_base != result.get("main_sha_before_race")
        and competitor.get("merge_commit_sha") == advanced_base
        and result.get("main_sha_after_target_attempt") is not None
    )
    if target.get("merged") is True and approved_competitor:
        return "stale_base_merge_accepted"
    expected_first_refusal = bool(attempts) and (
        attempts[0].get("label") == "immediate_after_base_advance"
        and attempts[0].get("http_status") == 405
        and attempts[0].get("reason") == "Please try again later"
    )
    verified_ready_readback = any(
        item.get("mergeable") is True
        and item.get("base_sha") == advanced_base
        and item.get("head_sha") == result.get("target_head_sha_at_last_read")
        and item.get("state") == "open"
        for item in readbacks
    )
    retry_reason = attempts[-1].get("reason", "") if attempts else ""
    exact_stale_base_reason = (
        retry_reason == "The head branch is behind the base branch"
        or retry_reason.endswith(
            "[reason: The head branch is behind the base branch]"
        )
    )
    expected_retry_refusal = bool(attempts) and (
        attempts[-1].get("label") == "after_forgejo_mergeability_readback"
        and attempts[-1].get("http_status") == 405
        and exact_stale_base_reason
    )
    stayed_unmerged_on_advanced_base = (
        target.get("state") == "open"
        and target.get("merged") is False
        and target.get("head_sha") == result.get("target_head_sha_at_last_read")
        and target.get("base_sha") == advanced_base
        and result.get("main_sha_after_target_attempt") == advanced_base
        and advanced_base != result.get("main_sha_before_race")
    )

    if (
        approved_competitor
        and expected_first_refusal
        and verified_ready_readback
        and expected_retry_refusal
        and stayed_unmerged_on_advanced_base
    ):
        return "stale_base_merge_refused"
    if (
        stayed_unmerged_on_advanced_base
        and attempts
        and attempts[-1].get("http_status", 0) >= 400
    ):
        return "stale_base_merge_refused_unclassified"
    return "inconsistent_readback"


def run(args: argparse.Namespace) -> dict[str, Any]:
    original_factory = driver.make_api
    calls = 0
    owner_api = None
    reviewer_api = None
    target_pr_number = None
    injection_done = False
    state = Path(args.state_dir)
    evidence = state / "evidence"

    def install_race(target_path: str, target_merge: dict[str, Any]) -> dict[str, Any]:
        nonlocal injection_done
        assert owner_api is not None and reviewer_api is not None
        injection_done = True
        premerge_evidence_names = (
            "final-premerge-preflight.json",
            "premerge-forgejo-readback.json",
            "reviews-final.json",
            "changed-files-before-merge.json",
        )
        premerge_evidence_hashes = {}
        for name in premerge_evidence_names:
            path = evidence / name
            if not path.is_file():
                raise RuntimeError(f"target final-check evidence missing before injection: {name}")
            premerge_evidence_hashes[name] = sha256(path.read_bytes())
        target_number = int(target_path.rsplit("/", 1)[-1])
        _, target_before, _ = owner_api("GET", target_path)
        expected_base = target_before.get("base", {}).get("sha")
        expected_head = target_before.get("head", {}).get("sha")
        expected_head_arg = target_merge.get("head_commit_id")
        if (
            target_before.get("state") != "open"
            or target_before.get("merged") is not False
            or not isinstance(expected_base, str)
            or not isinstance(expected_head, str)
            or expected_head_arg != expected_head
        ):
            raise RuntimeError("target PR is not in the expected verified state before injection")

        _, base_branch, _ = owner_api(
            "GET", f"/repos/{driver.OWNER}/{driver.REPOSITORY}/branches/main"
        )
        old_main = base_branch.get("commit", {}).get("id")
        if old_main != expected_base:
            raise RuntimeError("target PR base already moved before the race injection")

        competitor_branch = "race/base-advance"
        owner_api("POST", f"/repos/{driver.OWNER}/{driver.REPOSITORY}/branches", {
            "new_branch_name": competitor_branch,
            "old_branch_name": "main",
        })
        _, readme, _ = owner_api(
            "GET",
            f"/repos/{driver.OWNER}/{driver.REPOSITORY}/contents/README.md?ref=main",
        )
        old_readme = base64.b64decode(readme["content"])
        changed_readme = old_readme + b"\nBase advanced by the approved freshness-race fixture.\n"
        owner_api("PUT", f"/repos/{driver.OWNER}/{driver.REPOSITORY}/contents/README.md", {
            "message": "Advance protected base during candidate merge window",
            "content": base64.b64encode(changed_readme).decode("ascii"),
            "branch": competitor_branch,
            "sha": readme["sha"],
        })
        _, competitor_branch_readback, _ = owner_api(
            "GET", f"/repos/{driver.OWNER}/{driver.REPOSITORY}/branches/{competitor_branch}"
        )
        competitor_head = competitor_branch_readback["commit"]["id"]
        _, competitor_pr, _ = owner_api(
            "POST", f"/repos/{driver.OWNER}/{driver.REPOSITORY}/pulls", {
                "title": "Approved concurrent base advance",
                "body": "Local race fixture: advance main after the other PR's final verification reads.",
                "head": competitor_branch,
                "base": "main",
            }
        )
        competitor_number = int(competitor_pr["number"])
        competitor_path = f"/repos/{driver.OWNER}/{driver.REPOSITORY}/pulls/{competitor_number}"
        for _ in range(60):
            _, competitor_pr, _ = owner_api("GET", competitor_path)
            if competitor_pr.get("mergeable") is True:
                break
            driver.time.sleep(0.25)
        if competitor_pr.get("mergeable") is not True:
            raise RuntimeError("Forgejo did not mark the competing PR mergeable")
        if competitor_pr.get("head", {}).get("sha") != competitor_head:
            raise RuntimeError("competing PR head changed before review")
        _, competitor_review, _ = reviewer_api(
            "POST", competitor_path + "/reviews", {
                "body": "Independent fixture approval of the controlled base advance.",
                "event": "APPROVED",
                "commit_id": competitor_head,
            }
        )
        if (
            competitor_review.get("state") != "APPROVED"
            or competitor_review.get("commit_id") != competitor_head
        ):
            raise RuntimeError("Forgejo did not bind the concurrent approval to its head")
        competitor_review_evidence = {
            "id": competitor_review.get("id"),
            "state": competitor_review.get("state"),
            "commit_id": competitor_review.get("commit_id"),
            "reviewer_id": competitor_review.get("user", {}).get("id"),
            "reviewer_login": competitor_review.get("user", {}).get("login"),
            "submitted_at": competitor_review.get("submitted_at"),
        }
        competing_merge_status, competing_merge, _ = owner_api(
            "POST", competitor_path + "/merge", {
                "Do": "merge",
                "head_commit_id": competitor_head,
            }
        )
        _, competitor_after, _ = owner_api("GET", competitor_path)
        _, advanced_branch, _ = owner_api(
            "GET", f"/repos/{driver.OWNER}/{driver.REPOSITORY}/branches/main"
        )
        advanced_base = advanced_branch.get("commit", {}).get("id")
        if (
            competitor_after.get("merged") is not True
            or competitor_after.get("state") != "closed"
            or not isinstance(advanced_base, str)
            or advanced_base == old_main
        ):
            raise RuntimeError("the separately reviewed PR did not advance main")

        merge_attempts: list[dict[str, Any]] = []

        def attempt_target_merge(label: str) -> None:
            try:
                status, response, _ = owner_api(
                    "POST", target_path + "/merge", target_merge
                )
                merge_attempts.append({
                    "label": label,
                    "http_status": status,
                    "response": response,
                })
            except driver.ApiFailure as error:
                merge_attempts.append({
                    "label": label,
                    "http_status": error.status,
                    "reason": driver.api_error_message(error),
                })

        _, target_state_before_post, _ = owner_api("GET", target_path)
        attempt_target_merge("immediate_after_base_advance")
        first_attempt = merge_attempts[-1]
        readiness_history: list[dict[str, Any]] = []
        if (
            first_attempt["http_status"] == 405
            and first_attempt.get("reason") == "Please try again later"
        ):
            for _ in range(60):
                _, current_target, _ = owner_api("GET", target_path)
                readiness_history.append({
                    "mergeable": current_target.get("mergeable"),
                    "state": current_target.get("state"),
                    "base_sha": current_target.get("base", {}).get("sha"),
                    "head_sha": current_target.get("head", {}).get("sha"),
                })
                if current_target.get("mergeable") is True:
                    attempt_target_merge("after_forgejo_mergeability_readback")
                    break
                driver.time.sleep(0.25)

        _, target_after, _ = owner_api("GET", target_path)
        _, main_after, _ = owner_api(
            "GET", f"/repos/{driver.OWNER}/{driver.REPOSITORY}/branches/main"
        )
        current_main = main_after.get("commit", {}).get("id")
        merge_commit = target_after.get("merge_commit_sha")
        commit_info: dict[str, Any] | None = None
        if isinstance(merge_commit, str):
            _, commit_info, _ = owner_api(
                "GET", f"/repos/{driver.OWNER}/{driver.REPOSITORY}/git/commits/{merge_commit}"
            )
        source_readback_error = None
        merged_source_sha256 = None
        try:
            _, merged_source_doc, _ = owner_api(
                "GET", f"/repos/{driver.OWNER}/{driver.REPOSITORY}/contents/src/lib.rs?ref=main"
            )
            merged_source_sha256 = sha256(base64.b64decode(merged_source_doc["content"]))
        except driver.ApiFailure as error:
            source_readback_error = {
                "http_status": error.status,
                "reason": driver.api_error_message(error),
            }
        last_attempt = merge_attempts[-1] if merge_attempts else {}
        result = {
            "schema_version": 2,
            "scenario": "advance main through a separately approved PR after the target's final checks and immediately before its merge POST",
            "target_pull_request": target_number,
            "target_base_sha_at_last_read": expected_base,
            "target_head_sha_at_last_read": expected_head,
            "target_merge_request_head_sha": expected_head_arg,
            "concurrent_pull_request": competitor_number,
            "concurrent_pull_request_approved_head": competitor_head,
            "concurrent_pull_request_review": competitor_review_evidence,
            "concurrent_pull_request_merge_http_status": competing_merge_status,
            "concurrent_pull_request_merge_response": competing_merge,
            "concurrent_pull_request_after": {
                "state": competitor_after.get("state"),
                "merged": competitor_after.get("merged"),
                "head_sha": competitor_after.get("head", {}).get("sha"),
                "merge_commit_sha": competitor_after.get("merge_commit_sha"),
            },
            "main_sha_before_race": old_main,
            "main_sha_after_concurrent_merge_before_target_merge": advanced_base,
            "target_mergeability_before_post": target_state_before_post.get("mergeable"),
            "target_merge_attempts": merge_attempts,
            "mergeability_readbacks_after_initial_refusal": readiness_history,
            "target_pull_request_after_attempt": {
                "state": target_after.get("state"),
                "merged": target_after.get("merged"),
                "mergeable": target_after.get("mergeable"),
                "base_sha": target_after.get("base", {}).get("sha"),
                "head_sha": target_after.get("head", {}).get("sha"),
                "merge_commit_sha": target_after.get("merge_commit_sha"),
            },
            "main_sha_after_target_attempt": current_main,
            "target_premerge_evidence_sha256": premerge_evidence_hashes,
            "target_merge_commit": merge_commit,
            "target_merge_commit_parents": [
                item.get("sha") for item in (commit_info or {}).get("parents", [])
            ],
            "main_source_sha256_after_attempt": merged_source_sha256,
            "main_source_readback_error": source_readback_error,
            "main_readme_contains_race_marker": (
                b"Base advanced by the approved freshness-race fixture." in
                base64.b64decode(owner_api(
                    "GET", f"/repos/{driver.OWNER}/{driver.REPOSITORY}/contents/README.md?ref=main"
                )[1]["content"])
            ),
            "classification": "pending",
            "interpretation": "A merge accepted after main advanced means the proof's original base is no longer the exact merge base; a local/cooperative lock cannot exclude API writers that do not participate in it.",
            "limitation": "Disposable local Forgejo v17 binary. The concurrent writer is an ordinary approved PR merge; no production service or LLM is used.",
        }
        result["classification"] = classify_race_result(result)
        driver.write_json(evidence / "base-freshness-race.json", result)
        raise RaceCaptured

    def hooked_factory(token: str | None):
        nonlocal calls, owner_api, reviewer_api, target_pr_number
        calls += 1
        role = {1: "bootstrap", 2: "owner", 3: "reviewer"}.get(calls)
        if role is None:
            raise RuntimeError("unexpected API identity in the local Forgejo runner")
        raw_api = original_factory(token)

        def wrapped(method: str, path: str, data: Any = None):
            nonlocal injection_done, target_pr_number
            if (
                role == "owner"
                and reviewer_api is not None
                and target_pr_number is not None
                and not injection_done
                and method == "POST"
                and path == (
                    f"/repos/{driver.OWNER}/{driver.REPOSITORY}/pulls/"
                    f"{target_pr_number}/merge"
                )
            ):
                return install_race(path.removesuffix("/merge"), data or {})
            response = raw_api(method, path, data)
            if (
                role == "owner"
                and method == "POST"
                and path == f"/repos/{driver.OWNER}/{driver.REPOSITORY}/pulls"
                and isinstance(data, dict)
                and data.get("head") == "agent/clamp"
            ):
                target_pr_number = int(response[1]["number"])
            return response

        if role == "owner":
            owner_api = wrapped
        elif role == "reviewer":
            reviewer_api = wrapped
        return wrapped

    driver.make_api = hooked_factory
    try:
        driver.run(SimpleNamespace(
            state_dir=args.state_dir,
            forgejo_binary=args.forgejo_binary,
            repo_root=args.repo_root,
        ))
    except RaceCaptured:
        result = json.loads((evidence / "base-freshness-race.json").read_text())
        print(json.dumps(result, sort_keys=True))
        # The test passes only if Forgejo refuses after the approved base
        # advance. Acceptance is the reproduced freshness-gate violation.
        return result["classification"] == "stale_base_merge_refused"
    finally:
        driver.make_api = original_factory
    raise RuntimeError("target merge interception did not occur")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--repo-root", required=True)
    args = parser.parse_args()
    if run(args):
        return 0
    return 2


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"FAIL-CLOSED: {error}", file=__import__("sys").stderr)
        raise SystemExit(1)
