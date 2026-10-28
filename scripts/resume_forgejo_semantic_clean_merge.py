#!/usr/bin/env python3
"""Continue only the corrected-head phase of a retained local semantic PR canary."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import subprocess
import sys
import time

import run_forgejo_live_pr_merge_e2e as runner
import semantic_merge_canary as canary


def run(forgejo_binary: Path, state: Path, repo_root: Path) -> dict[str, object]:
    if not state.is_absolute() or state.parent != Path("/tmp") or not state.name.startswith(
        "forgejo-semantic-clean-merge-live-"
    ):
        raise RuntimeError("resume accepts only a retained semantic canary state directly under /tmp")
    if not state.is_dir() or state.is_symlink():
        raise RuntimeError("semantic canary state must be an existing non-symlink directory")
    evidence_dir = state / "evidence" / "semantic-clean-merge-live"
    required = [
        evidence_dir / "clean-merge-preview-binding.json",
        evidence_dir / "lower-no-review-binding.json",
        evidence_dir / "policy.json",
        evidence_dir / "requirements-lower.md",
        evidence_dir / "evidence" / "combined-failing-proof-proof-v2.json",
        evidence_dir / "evidence" / "combined-failing-proof-expected.json",
        state / "evidence" / "verifier.key",
        state / "rust-artifacts" / "gate",
        state / "rust-artifacts" / "forgejo-reconcile-local",
    ]
    if any(path.is_symlink() or not path.is_file() for path in required):
        raise RuntimeError("retained canary state is missing required regular evidence or binaries")
    if forgejo_binary.is_symlink() or not forgejo_binary.is_file():
        raise RuntimeError("Forgejo binary must be an existing non-symlink regular file")
    if repo_root.is_symlink() or not (repo_root / "Cargo.toml").is_file():
        raise RuntimeError("repo root is not the local Rust product")

    preview = json.loads((evidence_dir / "clean-merge-preview-binding.json").read_bytes())
    no_review = json.loads((evidence_dir / "lower-no-review-binding.json").read_bytes())
    if (
        preview.get("source_exactly_matches_branch_union") is not True
        or preview.get("preview_base_commit") != no_review.get("base_commit")
        or preview.get("preview_candidate_commit") != no_review.get("candidate_commit")
    ):
        raise RuntimeError("retained refusal evidence does not bind the preview and no-review binding")
    with socket.socket() as port_probe:
        if port_probe.connect_ex(("127.0.0.1", 3130)) == 0:
            raise RuntimeError("127.0.0.1:3130 is occupied; refusing to attach to an existing process")

    rustc_program = shutil.which("rustc")
    if rustc_program is None:
        raise RuntimeError("rustc is required to resume the protected acceptance")
    rust_env = {
        "PATH": f"{Path(rustc_program).parent}:/usr/bin:/bin",
        "HOME": str(Path.home()),
        "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
    }
    if os.environ.get("RUSTUP_TOOLCHAIN"):
        rust_env["RUSTUP_TOOLCHAIN"] = os.environ["RUSTUP_TOOLCHAIN"]
    sysroot_out = subprocess.run(
        [rustc_program, "--print", "sysroot"],
        cwd="/",
        env=rust_env,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        timeout=10,
    ).stdout.strip()
    rust_sysroot = Path(sysroot_out).resolve(strict=True)
    bwrap = "/usr/bin/bwrap"
    if not Path(bwrap).is_file() or Path(bwrap).is_symlink():
        raise RuntimeError("Bubblewrap is required to resume untrusted candidate acceptance")

    binary = forgejo_binary.resolve(strict=True)
    server_log_path = state / "log" / "resume-semantic.log"
    server_log = server_log_path.open("ab")
    server_environment = {
        "PATH": "/usr/bin:/bin",
        "HOME": str(state / "home"),
        "TMPDIR": str(state / "tmp"),
    }
    forgejo = subprocess.Popen(
        [str(binary), "--work-path", str(state), "--config", str(state / "custom/conf/app.ini"), "web"],
        cwd=state,
        env=server_environment,
        stdout=server_log,
        stderr=subprocess.STDOUT,
        close_fds=True,
    )
    try:
        runner.wait_for_health(forgejo)
        runner.assert_forgejo_listener(forgejo)
        admin_token = runner.call_admin(
            binary,
            state,
            [
                "admin", "user", "generate-access-token", "--username", runner.OWNER,
                "--token-name", f"semantic-resume-host-{secrets.token_hex(6)}", "--raw", "--scopes",
                "read:user,write:repository,write:issue",
            ],
        ).decode().strip()
        reviewer_token = runner.call_admin(
            binary,
            state,
            [
                "admin", "user", "generate-access-token", "--username", runner.REVIEWER,
                "--token-name", f"semantic-resume-reviewer-{secrets.token_hex(6)}", "--raw", "--scopes",
                "read:user,write:repository",
            ],
        ).decode().strip()
        if not admin_token or not reviewer_token or any(c.isspace() for c in admin_token + reviewer_token):
            raise RuntimeError("Forgejo returned an invalid local resume credential")
        api = runner.make_api(admin_token)
        reviewer_api = runner.make_api(reviewer_token)
        _, reviewer, _ = reviewer_api("GET", "/user")
        reviewer_id = reviewer.get("id")
        if reviewer.get("login") != runner.REVIEWER or not isinstance(reviewer_id, int):
            raise RuntimeError("Forgejo reviewer identity does not match the recorded review policy")

        client = state / "rust-artifacts" / "forgejo-reconcile-local"
        gate = state / "rust-artifacts" / "gate"
        key_path = state / "evidence" / "verifier.key"
        policy_path = evidence_dir / "policy.json"
        policy_bytes = policy_path.read_bytes()
        requirement = (evidence_dir / "requirements-lower.md").read_bytes()
        lower_number = int(no_review["pull_request_number"])
        lower_path = f"/repos/{runner.OWNER}/{runner.REPOSITORY}/pulls/{lower_number}"
        base_sha = str(preview["preview_base_commit"])
        old_head = str(preview["preview_candidate_commit"])
        branch = "agent/semantic-lower"
        _, current_main, _ = api("GET", f"/repos/{runner.OWNER}/{runner.REPOSITORY}/branches/main")
        _, current_pr, _ = api("GET", lower_path)
        _, branch_snapshot, _ = api(
            "GET", f"/repos/{runner.OWNER}/{runner.REPOSITORY}/branches/{runner.quote(branch, safe='')}"
        )
        fixture = canary.build_semantic_canary_sources()
        current_source, _ = runner.read_forgejo_file(api, "src/semantic.rs", branch)
        expected_corrected_source = fixture["corrected"]
        protected_acceptance = canary.semantic_acceptance_source("protected")
        corrected_binding_path = evidence_dir / "corrected-pr-binding.json"
        if current_pr.get("state") != "open" or current_pr.get("merged") is not False:
            raise RuntimeError("retained semantic PR is no longer open for correction and merge")
        if (
            current_main.get("commit", {}).get("id") != base_sha
            or current_pr.get("base", {}).get("sha") != base_sha
            or branch_snapshot.get("commit", {}).get("id") != current_pr.get("head", {}).get("sha")
        ):
            raise RuntimeError("retained Forgejo main, PR base and candidate branch are no longer bound")
        if current_pr.get("head", {}).get("sha") == old_head:
            if current_source != fixture["combined"]:
                raise RuntimeError("the retained candidate source no longer matches the red preview")
            runner.write_forgejo_file(
                api,
                path="src/semantic.rs",
                ref=branch,
                content=expected_corrected_source,
                message="Reject an empty effective range before clamping",
            )
            corrected_source, _ = runner.read_forgejo_file(api, "src/semantic.rs", branch)
            _, corrected_branch, _ = api(
                "GET", f"/repos/{runner.OWNER}/{runner.REPOSITORY}/branches/{runner.quote(branch, safe='')}"
            )
            corrected_head = corrected_branch.get("commit", {}).get("id")
            current_pr, corrected_observations = runner.wait_for_updated_pr_mergeability(
                api,
                lower_path,
                expected_head_sha=corrected_head,
                expected_base_sha=base_sha,
            )
            green_status, green_output = runner.run_semantic_acceptance_sandbox(
                corrected_source,
                protected_acceptance,
                state / "semantic-verify-protected-green-resumed",
                bwrap=bwrap,
                rust_sysroot=rust_sysroot,
            )
            runner.write_private(evidence_dir / "protected-green-acceptance.log", green_output)
            if (
                corrected_source != expected_corrected_source
                or current_pr.get("mergeable") is not True
                or green_status != 0
                or b"1 passed; 0 failed" not in green_output
            ):
                raise RuntimeError("the correction did not pass protected acceptance on the refreshed PR head")
            runner.write_json(
                corrected_binding_path,
                {
                    "base_commit": base_sha,
                    "candidate_commit": corrected_head,
                    "source_sha256": hashlib.sha256(corrected_source).hexdigest(),
                    "protected_acceptance_exit_code": green_status,
                    "mergeable": current_pr.get("mergeable"),
                    "mergeability_observations": corrected_observations,
                },
            )
        else:
            if corrected_binding_path.is_symlink() or not corrected_binding_path.is_file():
                raise RuntimeError("the resumed correction has no durable candidate binding")
            corrected_binding = json.loads(corrected_binding_path.read_bytes())
            corrected_head = corrected_binding.get("candidate_commit")
            corrected_source = current_source
            green_output_path = evidence_dir / "protected-green-acceptance.log"
            if green_output_path.is_symlink() or not green_output_path.is_file():
                raise RuntimeError("the resumed correction has no protected acceptance transcript")
            green_output = green_output_path.read_bytes()
            green_status = corrected_binding.get("protected_acceptance_exit_code")
            corrected_observations = corrected_binding.get("mergeability_observations")
            if (
                current_pr.get("head", {}).get("sha") != corrected_head
                or corrected_source != expected_corrected_source
                or hashlib.sha256(corrected_source).hexdigest() != corrected_binding.get("source_sha256")
                or current_pr.get("mergeable") is not True
                or green_status != 0
                or b"1 passed; 0 failed" not in green_output
            ):
                raise RuntimeError("retained correction evidence does not match the live PR head")

        final_evidence = runner.make_semantic_evidence(
            requirement=requirement,
            policy_bytes=policy_bytes,
            candidate_commit=corrected_head,
            base_commit=base_sha,
            source=corrected_source,
            acceptance_source=protected_acceptance,
            output=green_output,
            outcome="pass",
            detail="The independent protected acceptance passed on the corrected and refreshed Forgejo PR head.",
        )
        final_proof, final_expected, _, _ = runner.make_semantic_target_proof(
            state=evidence_dir,
            gate=gate,
            key_path=key_path,
            evidence=final_evidence,
            outcome="pass",
            name="corrected-proof",
        )
        review_binding = runner.create_review_binding(
            api=api,
            reviewer_api=reviewer_api,
            pr_path=lower_path,
            pull_number=lower_number,
            base_sha=base_sha,
            head_sha=corrected_head,
            reviewer_id=reviewer_id,
            evidence_dir=evidence_dir,
            name="corrected",
        )
        merge_status, merge_gate = runner.run_local_test_merge_authority(
            client,
            number=lower_number,
            forgejo_pid=forgejo.pid,
            target_profile="merge-014-agent-forge-core",
            proof=final_proof,
            expected=final_expected,
            key=key_path,
            policy=policy_path,
            review_base_binding=review_binding,
            execution_identity="local-test-executor",
            host_token=admin_token,
        )
        runner.write_json(evidence_dir / "corrected-merge-gate.json", merge_gate)
        if (
            merge_status != 0
            or merge_gate.get("verdict") != "merged"
            or merge_gate.get("merge_attempted") is not True
            or merge_gate.get("submitted_head_sha") != corrected_head
            or merge_gate.get("validated_base_sha") != base_sha
        ):
            raise RuntimeError("the trusted gate did not merge the corrected semantic candidate")

        _, final_pr, _ = api("GET", lower_path)
        _, final_main, _ = api("GET", f"/repos/{runner.OWNER}/{runner.REPOSITORY}/branches/main")
        issue_refs = re.findall(r"(?i)\bcloses\s+#([0-9]+)", str(final_pr.get("body", "")))
        if len(issue_refs) != 1:
            raise RuntimeError("the lower semantic PR did not retain one unambiguous issue link")
        issue_number = int(issue_refs[0])
        _, final_issue, _ = api("GET", f"/repos/{runner.OWNER}/{runner.REPOSITORY}/issues/{issue_number}")
        postmerge_source, _ = runner.read_forgejo_file(api, "src/semantic.rs", "main")
        postmerge_status, postmerge_output = runner.run_semantic_acceptance_sandbox(
            postmerge_source,
            protected_acceptance,
            state / "semantic-verify-protected-postmerge-resumed",
            bwrap=bwrap,
            rust_sysroot=rust_sysroot,
        )
        runner.write_private(evidence_dir / "postmerge-protected-acceptance.log", postmerge_output)
        if (
            final_pr.get("state") != "closed"
            or final_pr.get("merged") is not True
            or final_pr.get("head", {}).get("sha") != corrected_head
            or final_pr.get("merge_commit_sha") != merge_gate.get("merge_commit_sha")
            or final_main.get("commit", {}).get("id") != merge_gate.get("merge_commit_sha")
            or final_issue.get("state") != "closed"
            or postmerge_source != corrected_source
            or postmerge_status != 0
            or b"1 passed; 0 failed" not in postmerge_output
        ):
            raise RuntimeError("post-merge Forgejo state or behavior differs from the verified correction")

        result = {
            "schema_version": 1,
            "continued_existing_run": True,
            "local_fixture_only": True,
            "llm_invoked": False,
            "pull_request_number": lower_number,
            "initial_failing_candidate": old_head,
            "corrected_candidate": corrected_head,
            "base_commit": base_sha,
            "stale_proof_retested": False,
            "corrected_gate_verdict": merge_gate.get("verdict"),
            "merge_commit": merge_gate.get("merge_commit_sha"),
            "postmerge_acceptance": "passed",
            "issue_closed": final_issue.get("state") == "closed",
            "limitations": [
                "Fixture verifier/local-test-adapter only; no LLM, AgentOS, conventional CI, or productivity benchmark.",
                "The initial runner stopped after its assertion expected the wrong public error code; the retained gate result was a valid `verification_failed` refusal with no merge attempt.",
                "The test does not exclude external Forgejo writers or multi-tenant/kernel attacks.",
            ],
        }
        runner.write_json(evidence_dir / "resume-result.json", result)
        print(json.dumps(result, sort_keys=True))
        return result
    finally:
        if forgejo.poll() is None:
            forgejo.terminate()
            try:
                forgejo.wait(timeout=10)
            except subprocess.TimeoutExpired:
                forgejo.kill()
                forgejo.wait(timeout=5)
        server_log.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--forgejo-binary", type=Path, required=True)
    parser.add_argument("--state-dir", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    try:
        run(args.forgejo_binary, args.state_dir, args.repo_root)
    except Exception as error:
        print(f"FAIL-CLOSED: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
