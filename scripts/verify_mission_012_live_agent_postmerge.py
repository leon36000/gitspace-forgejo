#!/usr/bin/env python3
"""Recheck a preserved private MISSION-012 Forgejo merge without credentials."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import stat
import sys

import run_mission_012_live_agent_pr_e2e as driver


def read_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_bytes())
    if not isinstance(value, dict):
        raise RuntimeError(f"expected a JSON object in {path.name}")
    return value


def write_bytes_new(path: Path, value: bytes) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    with os.fdopen(fd, "wb") as stream:
        stream.write(value)
        stream.flush()
        os.fsync(stream.fileno())


def run(state_argument: str, evidence_argument: str, admission_argument: str, run_id: str) -> dict[str, object]:
    if not re.fullmatch(r"[a-z0-9-]{1,24}", run_id):
        raise RuntimeError("run ID must be a short lowercase identifier")
    state_path = Path(state_argument)
    if state_path.is_symlink():
        raise RuntimeError("private state directory cannot be a symbolic link")
    state = state_path.resolve(strict=True)
    evidence = Path(evidence_argument).resolve(strict=True)
    admission_path = Path(admission_argument)
    candidate_root = state / "project/.runtime/live-mcp-005"
    if (
        state.parent != Path("/tmp") or stat.S_IMODE(state.stat().st_mode) != 0o700
        or candidate_root.is_symlink() or not candidate_root.is_dir()
        or admission_path.is_symlink() or not admission_path.is_absolute()
        or admission_path.parent != state / "project/.runtime"
        or stat.S_IMODE(admission_path.stat().st_mode) != 0o600
    ):
        raise RuntimeError("preserved Forgejo state or host admission is outside the private fixture")

    result_dir = evidence / "mission-012-live-agent-opus-005"
    result = read_json(result_dir / "forgejo-result.json")
    final = read_json(result_dir / "forgejo-final-readback.json")
    verifier_dir = candidate_root / "trusted-repository"
    bare_repo = state / "repositories/forge-admin/mission-012-clamp.git"
    if any(path.is_symlink() or not path.is_dir() for path in (verifier_dir, bare_repo)):
        raise RuntimeError("preserved Git repositories are missing or redirected")

    summary = read_json(candidate_root / "model-run-summary.json")
    admission_raw = admission_path.read_bytes()
    admission = json.loads(admission_raw)
    if (
        summary.get("admission_sha256") != driver.digest(admission_raw)
        or admission.get("base_commit") != result.get("base_commit")
        or admission.get("allocation", {}).get("execution_generation") != summary.get("execution_generation")
        or summary.get("base_commit") != result.get("base_commit")
        or summary.get("candidate_file_sha256") != {"src/lib.rs": result.get("candidate_source_sha256")}
    ):
        raise RuntimeError("admission, model summary, and preserved Forgejo result do not bind")
    receipt_sha = driver.verify_host_receipt(
        summary, candidate_root, driver.execution_binding_from_admission(admission)
    )
    if receipt_sha != result.get("host_receipt_sha256"):
        raise RuntimeError("host receipt hash differs from the preserved Forgejo result")

    base = str(result.get("base_commit", ""))
    candidate = str(result.get("candidate_commit", ""))
    merge_commit = str(result.get("merge_commit", ""))
    main_sha = str(result.get("main_head_after_merge", ""))
    for value in (base, candidate, merge_commit, main_sha):
        if not re.fullmatch(r"[0-9a-f]{40}", value):
            raise RuntimeError("preserved Forgejo result contains an invalid commit id")
    if (
        result.get("merged") is not True or result.get("pull_request_state_after_merge") != "closed"
        or result.get("issue_state_after_merge") != "closed" or result.get("changed_paths") != ["src/lib.rs"]
        or final.get("read_only") is not True or final.get("repository_private") is not True
        or final.get("main_sha") != main_sha or final.get("main_sha") != merge_commit
        or final.get("main_source_sha256") != result.get("candidate_source_sha256")
        or final.get("issue_state") != "closed"
    ):
        raise RuntimeError("preserved Forgejo API read-back does not describe the expected private merge")
    pull = final.get("pull_request", {})
    review = final.get("review", {})
    protection = final.get("branch_protection", {})
    if (
        pull.get("number") != result.get("pull_request_number") or pull.get("state") != "closed"
        or pull.get("merged") is not True or pull.get("base_sha") != main_sha
        or pull.get("head_sha") != candidate or pull.get("merge_commit_sha") != merge_commit
        or final.get("review_count") != 1 or final.get("review_total_count") != 1
        or review.get("state") != "APPROVED" or review.get("commit_id") != candidate
        or review.get("official") is not True or review.get("stale") is not False
        or review.get("dismissed") is not False or review.get("reviewer_login") != "verifier-bot"
        or protection.get("branch_name") != "main" or protection.get("required_approvals") != 1
        or protection.get("approvals_whitelist_username") != ["verifier-bot"]
        or protection.get("merge_whitelist_usernames") != ["forge-admin"]
        or protection.get("block_on_outdated_branch") is not True
    ):
        raise RuntimeError("preserved review or branch-protection read-back does not bind to the candidate")

    candidate_ref = f"refs/mission012/postmerge-candidate-{result_dir.name}"
    main_ref = f"refs/mission012/postmerge-main-{result_dir.name}"
    driver.git(["fetch", "--no-tags", str(bare_repo), f"refs/heads/agent/{result_dir.name}:{candidate_ref}"], repo=verifier_dir)
    driver.git(["fetch", "--no-tags", str(bare_repo), f"refs/heads/main:{main_ref}"], repo=verifier_dir)
    if (
        driver.git(["rev-parse", f"{candidate_ref}^"], repo=verifier_dir).decode().strip() != base
        or driver.git(["rev-parse", candidate_ref], repo=verifier_dir).decode().strip() != candidate
        or driver.git(["rev-parse", main_ref], repo=verifier_dir).decode().strip() != main_sha
    ):
        raise RuntimeError("preserved Git refs differ from the Forgejo API read-back")

    base_snapshot = state / f"verify-base-hardened-{run_id}"
    candidate_snapshot = state / f"verify-candidate-hardened-{run_id}"
    for target in (base_snapshot, candidate_snapshot):
        if target.exists() or target.is_symlink():
            raise RuntimeError("post-merge verification snapshot destination must be new")
    base_files = driver.materialize(verifier_dir, base, base_snapshot)
    candidate_files = driver.materialize(verifier_dir, candidate, candidate_snapshot)
    changed_paths = sorted(name for name in driver.EXPECTED_FILES if base_files[name] != candidate_files[name])
    if changed_paths != ["src/lib.rs"]:
        raise RuntimeError("historical candidate changed a pinned or protected acceptance file")
    driver.verify_pinned_sources(base_files["src/lib.rs"], base_files["tests/acceptance.rs"], base=True)
    driver.verify_pinned_sources(candidate_files["src/lib.rs"], candidate_files["tests/acceptance.rs"])

    rust_sysroot = Path("/home/pc6/.rustup/toolchains/1.95.0-aarch64-unknown-linux-gnu").resolve(strict=True)
    red_status, red_output = driver.run_retrospective_cargo_test(base_snapshot, rust_sysroot)
    if (
        red_status == 0 or b"values_outside_the_range_clamp_to_the_nearest_boundary" not in red_output
        or b"1 passed; 1 failed" not in red_output
    ):
        raise RuntimeError("hardened post-merge base test did not reproduce the pinned defect")
    green_status, green_output = driver.run_retrospective_cargo_test(candidate_snapshot, rust_sysroot)
    if green_status or b"2 passed; 0 failed" not in green_output:
        raise RuntimeError("hardened post-merge candidate test did not pass")

    parents = driver.git(["show", "-s", "--format=%P", merge_commit], repo=verifier_dir).decode().split()
    merge_tree = driver.git(["rev-parse", f"{merge_commit}^{{tree}}"], repo=verifier_dir).decode().strip()
    candidate_tree = driver.git(["rev-parse", f"{candidate}^{{tree}}"], repo=verifier_dir).decode().strip()
    if not driver.merge_parents_match(parents, base, candidate) or merge_tree != candidate_tree:
        raise RuntimeError("merge commit parents or tree do not match the tested PR candidate")

    verifier_path = result_dir / f"hardened-independent-verifier-{run_id}.json"
    base_log_path = result_dir / f"hardened-base-red-{run_id}.log"
    candidate_log_path = result_dir / f"hardened-candidate-green-{run_id}.log"
    driver.write_json(verifier_path, {
        "schema_version": 1,
        "verdict": "pass_retrospective",
        "pre_merge_gate": False,
        "candidate_commit": candidate,
        "merge_commit": merge_commit,
        "base_commit": base,
        "pull_request_number": result.get("pull_request_number"),
        "issue_number": result.get("issue_number"),
        "admission_sha256": driver.digest(admission_raw),
        "requirement_sha256": admission.get("requirement_sha256"),
        "policy_sha256": driver.POLICY_SHA,
        "verifier_id": "verify_mission_012_live_agent_postmerge.py/local-isolated-cargo-acceptance-v1",
        "verification_commands": ["cargo test --offline --locked"],
        "toolchain": str(rust_sysroot / "bin/cargo"),
        "merge_parents": parents,
        "candidate_tree_matches_merge_tree": True,
        "candidate_source_sha256": driver.digest(candidate_files["src/lib.rs"]),
        "acceptance_sha256": driver.digest(candidate_files["tests/acceptance.rs"]),
        "host_receipt_sha256_verified": receipt_sha,
        "base": {"exit_code": red_status, "output_sha256": driver.digest(red_output), "expected_failure": True},
        "candidate": {"exit_code": green_status, "output_sha256": driver.digest(green_output), "acceptance_pass": True},
        "sandbox": "systemd MemoryMax/TasksMax/CPUQuota/RuntimeMaxSec plus Bubblewrap user/mount/PID/network namespaces; commit snapshot read-only; fresh tmpfs Cargo target; no host home or credentials; offline locked Cargo",
        "mission_cpu_ledger": "not updated: this is a post-merge retrospective after the mission lease may have expired, and it has no pre-merge authority",
        "model_and_cost_provenance": "Claude CLI model, effort, and cost remain values reported by the local run summary; the host receipt hash, execution-binding consistency, and candidate bytes were independently checked. The local receipt is not a signed attestation and is not protected against a writer with the same UID.",
        "limitation": "This verification ran after merge. It independently rechecks the exact merged bytes and does not retroactively make the original pre-merge test-output parser resistant to arbitrary candidates. The hardened runner pins this task's exact source and acceptance hashes, requires the outdated-branch gate, and checks exact merge parents for future runs.",
    })
    write_bytes_new(base_log_path, red_output)
    write_bytes_new(candidate_log_path, green_output)
    return {
        "verdict": "pass_retrospective", "base_commit": base,
        "candidate_commit": candidate, "merge_commit": merge_commit,
        "source_sha256": driver.digest(candidate_files["src/lib.rs"]),
        "receipt_sha256": receipt_sha, "evidence": str(verifier_path),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    parser.add_argument("--admission-file", required=True)
    parser.add_argument("--run-id", default="001")
    args = parser.parse_args()
    try:
        print(json.dumps(run(args.state_dir, args.evidence_dir, args.admission_file, args.run_id), sort_keys=True))
    except Exception as error:
        print(f"FAIL-CLOSED: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
