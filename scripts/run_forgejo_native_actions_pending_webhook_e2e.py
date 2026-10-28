#!/usr/bin/env python3
"""Exercise a Forgejo-native pending Actions run through the Rust webhook adapter.

The instance, repository, workflow, webhook and credentials are disposable and
local. No runner is registered or launched, so this proves queued-state
reconciliation only; it cannot prove job execution or CI success.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import secrets
import socket
import subprocess
import sys
import time
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_webhook_reconcile_emitted as emitted  # noqa: E402
import run_forgejo_live_webhook_reconcile_e2e as reconcile  # noqa: E402
import run_forgejo_live_pr_merge_e2e as forgejo  # noqa: E402


OWNER = "forge-admin"
REPOSITORY = reconcile.REPOSITORY
READER = "forge-reader"
ORIGIN = reconcile.ORIGIN
FORGEJO_VERSION = "17.0.0+gitea-1.22.0"
EVENT_STATUS = {
    "workflow_run_waiting": "waiting",
    "workflow_run_blocked": "blocked",
    "workflow_run_running": "running",
    "workflow_run_success": "success",
    "workflow_run_failure": "failure",
    "workflow_run_cancelled": "cancelled",
    "workflow_run_skipped": "skipped",
}
ALL_ACTION_EVENTS = [
    *EVENT_STATUS,
    "workflow_job_blocked",
    "workflow_job_cancelled",
    "workflow_job_failure",
    "workflow_job_running",
    "workflow_job_skipped",
    "workflow_job_success",
    "workflow_job_waiting",
]
DELIVERY_RE = re.compile(r"^[A-Za-z0-9_-]{1,128}$")
SHA_RE = re.compile(r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$")


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def validate_native_run_delivery(
    record: dict[str, Any], *, run_id: int, run_index: int, commit_sha: str
) -> dict[str, Any]:
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("native run event has an invalid journal sequence")
    kind = record.get("kind")
    prefix = "forgejo.webhook.v1."
    if not isinstance(kind, str) or not kind.startswith(prefix):
        raise ValueError("record is not a supported workflow_run event")
    event = kind.removeprefix(prefix)
    if event not in EVENT_STATUS:
        raise ValueError("record is not a supported workflow_run event")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("native run event has no receiver envelope")
    delivery_id = envelope.get("delivery_id")
    if (
        envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != event.removeprefix("forgejo.webhook.v1.")
        or not isinstance(delivery_id, str)
        or not DELIVERY_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
    ):
        raise ValueError("native run delivery binding is invalid")
    body = envelope.get("payload")
    run = body.get("run") if isinstance(body, dict) else None
    if not isinstance(run, dict):
        raise ValueError("native run event has no run object")
    if (
        run.get("id") != run_id
        or run.get("index_in_repo") != run_index
        or run.get("status") != EVENT_STATUS[event]
        or run.get("commit_sha") != commit_sha
    ):
        raise ValueError("native run event does not match dispatched run identity/status")
    repository = run.get("repository")
    if not isinstance(repository, dict) or repository.get("full_name") != f"{OWNER}/{REPOSITORY}":
        raise ValueError("native run event does not name the disposable repository")
    action = body.get("action")
    if action not in {"new_run_attempt", "run_status_changed", "run_completed"}:
        raise ValueError("native run event contains an unsupported action")
    if EVENT_STATUS[event] in {"waiting", "blocked"} and action != "new_run_attempt":
        raise ValueError("initial pending run event is not a new_run_attempt")
    return {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": event.removeprefix("forgejo.webhook.v1."),
        "action": action,
        "status_at_event": EVENT_STATUS[event],
        "run_id": run_id,
        "run_index": run_index,
        "commit_sha": commit_sha,
    }


def wait_for_native_run_event(
    *, journal: Path, database: Path, hook_id: int, run_id: int,
    run_index: int, commit_sha: str, timeout: float = 30.0,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        records = emitted.read_journal_records(journal)
        tasks = emitted.hook_task_readback(database, hook_id)
        task_by_uuid = {task.get("uuid"): task for task in tasks}
        for record in records:
            envelope = record.get("payload")
            if not isinstance(envelope, dict):
                continue
            body = envelope.get("payload")
            run = body.get("run") if isinstance(body, dict) else None
            if not isinstance(run, dict) or run.get("id") != run_id:
                continue
            event = validate_native_run_delivery(
                record, run_id=run_id, run_index=run_index, commit_sha=commit_sha
            )
            task = task_by_uuid.get(event["delivery_id"])
            if (
                not isinstance(task, dict)
                or task.get("event_type") != event["event"]
                or task.get("is_delivered") is not True
                or task.get("is_succeed") is not True
                or task.get("response_status") != 202
            ):
                continue
            return event, tasks
        time.sleep(0.1)
    raise TimeoutError("Forgejo did not deliver a native workflow_run event to the receiver")


def create_user(binary: Path, state: Path, username: str, *, admin: bool) -> None:
    command = [
        "admin", "user", "create", "--username", username,
        "--random-password", "--email", f"{username}@local.invalid",
        "--must-change-password=false",
    ]
    if admin:
        command.append("--admin")
    reconcile.admin_command(binary, state, command)


def start_forgejo(binary: Path, state: Path, output):
    return subprocess.Popen(
        [str(binary), "--work-path", str(state), "--config", str(state / "custom/conf/app.ini"), "web"],
        cwd=state,
        env=reconcile.private_command_environment(state),
        stdin=subprocess.DEVNULL,
        stdout=output,
        stderr=subprocess.STDOUT,
        close_fds=True,
    )


def run(args: argparse.Namespace) -> dict[str, Any]:
    binary_arg = Path(args.forgejo_binary)
    if binary_arg.is_symlink():
        raise RuntimeError("Forgejo binary must not be a symlink")
    binary = binary_arg.resolve(strict=True)
    repo = Path(args.repo_root).resolve(strict=True)
    state = Path(args.state_dir)
    evidence = Path(args.evidence_dir).resolve()
    if not binary.is_file() or not (repo / "Cargo.toml").is_file():
        raise RuntimeError("local Forgejo binary or Rust product checkout is unavailable")
    if not state.is_absolute() or state.parent != Path("/tmp") or state.exists():
        raise RuntimeError("state dir must be a new direct child of /tmp")
    if evidence.exists() or evidence.is_symlink():
        raise RuntimeError("refusing to overwrite the evidence directory")
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", 3130)) == 0:
            raise RuntimeError("Forgejo port 3130 is already occupied")
    version = subprocess.run(
        [str(binary), "--version"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, env={"PATH": "/usr/bin:/bin", "HOME": "/tmp"}, timeout=10, check=True,
    ).stdout.strip()
    if FORGEJO_VERSION not in version:
        raise RuntimeError("expected the already available Forgejo 17.0.0 local binary")

    state.mkdir(mode=0o700)
    (state / "home").mkdir(mode=0o700)
    (state / "tmp").mkdir(mode=0o700)
    (state / "log").mkdir(mode=0o700)
    evidence.mkdir(mode=0o700, parents=True)
    forgejo.create_app_ini(state, allow_loopback_webhooks=True, actions_enabled=True)
    config = (state / "custom/conf/app.ini").read_text(encoding="utf-8")
    if "[actions]\nENABLED = true\n" not in config or "[webhook]\nALLOWED_HOST_LIST = 127.0.0.1\n" not in config:
        raise RuntimeError("test instance config did not enable Actions and restrict webhook delivery to loopback")

    forgejo_log = (state / "log/forgejo.log").open("wb")
    receiver_log = (state / "log/receiver.log").open("wb")
    tokens: list[str] = []
    server = None
    receiver = None
    receiver_port: int | None = None
    hook_id: int | None = None
    owner_api = None
    reader_api = None
    receiver_stop: dict[str, Any] | None = None
    server_stop: dict[str, Any] | None = None
    result: dict[str, Any] = {"status": "failed", "stage": "startup"}
    stage = "isolated_build"
    stage_facts: dict[str, Any] = {}
    try:
        receiver_bin, reconcile_bin, build = reconcile.build_local_binaries(repo, state, evidence)
        stage = "start_local_forgejo"
        server = start_forgejo(binary, state, forgejo_log)
        forgejo.wait_for_health(server)
        forgejo.assert_forgejo_listener(server)

        stage = "provision_private_repository"
        create_user(binary, state, OWNER, admin=True)
        bootstrap = reconcile.create_access_token(
            binary, state, OWNER, "actions-pending-bootstrap", "write:user,write:repository"
        )
        tokens.append(bootstrap)
        bootstrap_api = reconcile.LocalForgejoApi(bootstrap)
        created_repo, _ = bootstrap_api.request(
            "POST", "/user/repos",
            {"name": REPOSITORY, "private": True, "auto_init": True, "default_branch": "main"},
        )
        if created_repo.get("full_name") != f"{OWNER}/{REPOSITORY}" or created_repo.get("private") is not True:
            raise RuntimeError("Forgejo did not create the private test repository")
        tokens.remove(bootstrap)
        bootstrap = None
        bootstrap_api = None

        owner_token = reconcile.create_access_token(
            binary, state, OWNER, "actions-pending-owner",
            "read:user,read:repository,write:repository",
        )
        tokens.append(owner_token)
        owner_api = reconcile.LocalForgejoApi(owner_token)

        create_user(binary, state, READER, admin=False)
        reader_token = reconcile.create_access_token(
            binary, state, READER, "actions-pending-read-only", "read:user,read:repository"
        )
        tokens.append(reader_token)
        reader_api = reconcile.LocalForgejoApi(reader_token)
        owner_api.request("PUT", f"/repos/{OWNER}/{REPOSITORY}/collaborators/{READER}", {"permission": "read"})
        reader_repo, _ = reader_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}")
        permissions = reader_repo.get("permissions") if isinstance(reader_repo, dict) else None
        if (
            reader_repo.get("private") is not True or not isinstance(permissions, dict)
            or permissions.get("admin") is not False or permissions.get("push") is not False
            or permissions.get("pull") is not True
        ):
            raise RuntimeError("Forgejo did not confirm the read-only verifier account")

        stage = "add_and_read_back_workflow"
        workflow = (
            "name: Pending local adapter check\n"
            "on:\n"
            "  workflow_dispatch:\n"
            "jobs:\n"
            "  pending:\n"
            "    runs-on: forgejo-native-no-runner\n"
            "    steps:\n"
            "      - run: echo this job must remain queued\n"
        ).encode("utf-8")
        path = "/repos/{}/{}/contents/.forgejo/workflows/pending.yml".format(OWNER, REPOSITORY)
        created_file, _ = owner_api.request(
            "POST", path,
            {"message": "Add pending Actions workflow fixture", "content": base64.b64encode(workflow).decode("ascii"), "branch": "main"},
        )
        file_sha = created_file.get("content", {}).get("sha") if isinstance(created_file, dict) else None
        readback_file, _ = owner_api.request("GET", path + "?ref=main")
        readback_content = base64.b64decode(readback_file.get("content", ""), validate=True)
        if not isinstance(file_sha, str) or not file_sha or readback_content != workflow:
            raise RuntimeError("Forgejo workflow file readback did not match the submitted fixture")

        stage = "start_receiver_and_register_webhook"
        receiver_port = reconcile.select_receiver_port()
        receiver_address = f"127.0.0.1:{receiver_port}"
        webhook_secret = secrets.token_hex(32)
        secret_path = state / "webhook.secret"
        forgejo.write_private(secret_path, webhook_secret.encode("ascii"))
        attestation_seed = state / "receiver-attestation.seed"
        forgejo.write_private(attestation_seed, secrets.token_bytes(32))
        public_key = reconcile.derive_receiver_attestation_public_key(receiver_bin, state, attestation_seed)
        forgejo.write_private(state / "receiver-attestation.public", public_key.encode("ascii"))
        journal = state / "data/webhook-inbox.jsonl"
        receiver = subprocess.Popen(
            [str(receiver_bin), receiver_address, str(secret_path), str(journal), str(attestation_seed)],
            cwd=state, env=reconcile.private_command_environment(state), stdin=subprocess.DEVNULL,
            stdout=receiver_log, stderr=subprocess.STDOUT, close_fds=True,
        )
        reconcile.wait_for_listener(receiver, receiver_port)
        hook, _ = owner_api.request(
            "POST", f"/repos/{OWNER}/{REPOSITORY}/hooks",
            {
                "type": "gitea",
                "config": {"url": f"http://{receiver_address}/forgejo/webhook", "content_type": "json", "secret": webhook_secret},
                "events": ALL_ACTION_EVENTS,
                "active": True,
            },
        )
        hook_id = hook.get("id")
        if not isinstance(hook_id, int) or hook_id < 1 or hook.get("active") is not True:
            raise RuntimeError("Forgejo did not create the active local Actions webhook")
        hook_readback, _ = owner_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}")
        hook_events = hook_readback.get("events") if isinstance(hook_readback, dict) else None
        if not isinstance(hook_events, list) or set(hook_events) != set(ALL_ACTION_EVENTS):
            raise RuntimeError("Forgejo webhook event subscription failed readback")

        stage = "dispatch_native_workflow"
        dispatch, _ = owner_api.request(
            "POST", f"/repos/{OWNER}/{REPOSITORY}/actions/workflows/pending.yml/dispatches",
            {"ref": "main", "inputs": {}, "return_run_info": True},
        )
        if owner_api.last_status != 201 or not isinstance(dispatch, dict):
            raise RuntimeError("Forgejo did not return the dispatched workflow run")
        run_id = dispatch.get("id")
        run_index = dispatch.get("run_number")
        if not isinstance(run_id, int) or run_id < 1 or not isinstance(run_index, int) or run_index < 1:
            raise RuntimeError("Forgejo dispatch response has invalid run identity")

        stage = "read_current_run_as_read_only_account"
        current_run, _ = reader_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}/actions/runs/{run_id}")
        if (
            current_run.get("id") != run_id or current_run.get("index_in_repo") != run_index
            or current_run.get("status") not in {"waiting", "blocked"}
        ):
            raise RuntimeError("read-only API did not observe the run in a nonterminal pending state")
        commit_sha = current_run.get("commit_sha")
        if not isinstance(commit_sha, str) or not SHA_RE.fullmatch(commit_sha):
            raise RuntimeError("current API run has no valid commit SHA")
        stage_facts["dispatch"] = {
            "http_status": owner_api.last_status,
            "run_id": run_id,
            "run_index": run_index,
            "current_status_before_reconcile": current_run.get("status"),
            "commit_sha": commit_sha,
        }

        stage = "receive_and_bind_native_webhook"
        event, tasks = wait_for_native_run_event(
            journal=journal, database=state / "data/forgejo.db", hook_id=hook_id,
            run_id=run_id, run_index=run_index, commit_sha=commit_sha,
        )
        event_name = event["event"]
        delivery_id = event["delivery_id"]
        stage_facts["native_webhook"] = event
        stage_facts["matching_successful_hook_tasks"] = len(
            [task for task in tasks if task.get("uuid") == delivery_id and task.get("is_succeed") is True]
        )
        stage = "reconcile_native_delivery"
        cli_status, reconciled = reconcile.run_reconcile(
            reconcile_bin, state, journal, delivery_id, reader_token,
            trusted_attestation_public_key=public_key,
        )
        canonical = reconciled.get("canonical_state") if isinstance(reconciled, dict) else None
        stage_facts["reconciliation_diagnostic"] = {
            "exit_status": cli_status,
            "verdict": reconciled.get("verdict") if isinstance(reconciled, dict) else None,
            "upstream_read_attempts": reconciled.get("upstream_read_attempts") if isinstance(reconciled, dict) else None,
            "resource": reconciled.get("resource") if isinstance(reconciled, dict) else None,
            "canonical_status": canonical.get("state") if isinstance(canonical, dict) else None,
            "error": reconciled.get("error") if isinstance(reconciled, dict) else None,
        }
        if (
            cli_status != 0 or reconciled.get("verdict") != "current_state_read"
            or reconciled.get("upstream_read_attempts") != 1
            or reconciled.get("resource", {}).get("kind") != "workflow_run"
            or reconciled.get("resource", {}).get("run_id") != run_id
            or not isinstance(canonical, dict) or canonical.get("state") not in {"waiting", "blocked"}
            or canonical.get("head_sha") != commit_sha
        ):
            raise RuntimeError("Rust reconciler did not return the current pending run state")

        result = {
            "status": "passed",
            "forgejo_version": version,
            "forgejo_binary_sha256": file_sha256(binary),
            "forgejo_build_commit": "unknown",
            "forgejo_source_commit": "0d791421f85ca13a63b665053d81ea765fbbc571",
            "actions_enabled": True,
            "runner_registered_or_started_by_harness": False,
            "workflow_path": ".forgejo/workflows/pending.yml",
            "workflow_file_git_blob_sha": file_sha,
            "dispatch_http_status": owner_api.last_status,
            "run_identity": {"id": run_id, "index_in_repo": run_index, "commit_sha": commit_sha},
            "native_webhook": event,
            "hook_task_count": len(tasks),
            "hook_task_http_202_confirmed": True,
            "receiver_hmac_admission": "accepted_and_attested",
            "verifier_account": {"login": READER, "is_admin": False, "push": False, "pull": True},
            "reconciliation": {
                "exit_status": cli_status,
                "verdict": reconciled.get("verdict"),
                "upstream_read_attempts": reconciled.get("upstream_read_attempts"),
                "resource_kind": reconciled["resource"]["kind"],
                "run_id": run_id,
                "current_status": canonical["state"],
                "head_sha": canonical["head_sha"],
                "independent_verification": False,
                "merge_authorizing": False,
            },
            "not_proven": [
                "runner execution or successful CI",
                "AgentOS or LLM execution",
                "merge authority",
                "production security or upstream binary build commit",
            ],
            "build_provenance": build,
        }
        return result
    except Exception as error:
        result = {
            "status": "failed",
            "stage": stage,
            "error_type": type(error).__name__,
            "stage_facts": stage_facts,
        }
        raise
    finally:
        if hook_id is not None and owner_api is not None:
            try:
                owner_api.request(
                    "PATCH", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
                    {"active": False, "events": ALL_ACTION_EVENTS},
                )
                readback, _ = owner_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}")
                if not isinstance(readback, dict) or readback.get("active") is not False:
                    result["hook_deactivation_confirmed"] = False
                else:
                    result["hook_deactivation_confirmed"] = True
            except Exception:
                result["hook_deactivation_confirmed"] = False
        receiver_stop = reconcile.stop_process(receiver)
        server_stop = reconcile.stop_process(server)
        cleanup = {
            "receiver": receiver_stop,
            "forgejo": server_stop,
            "receiver_listener_after_stop": reconcile.listener_addresses_for_port(receiver_port) if receiver_port else [],
            "forgejo_listener_after_stop": reconcile.listener_addresses_for_port(3130),
        }
        cleanup["confirmed"] = (
            cleanup["receiver"]["stopped"] and cleanup["forgejo"]["stopped"]
            and not cleanup["receiver_listener_after_stop"] and not cleanup["forgejo_listener_after_stop"]
        )
        result["process_cleanup"] = cleanup
        result["private_values_recorded"] = False
        result["evidence_source_hashes"] = {
            "scripts/run_forgejo_native_actions_pending_webhook_e2e.py": file_sha256(Path(__file__).resolve()),
            "scripts/run_forgejo_live_pr_merge_e2e.py": file_sha256(Path(forgejo.__file__).resolve()),
            "scripts/run_forgejo_live_webhook_reconcile_e2e.py": file_sha256(Path(reconcile.__file__).resolve()),
            "scripts/run_forgejo_live_webhook_reconcile_emitted.py": file_sha256(Path(emitted.__file__).resolve()),
        }
        if evidence.exists():
            reconcile.write_safe_json(evidence / "result.json", result, tokens)
            if 'build' in locals():
                reconcile.write_safe_json(evidence / "build-provenance.json", build, tokens)
            reconcile.write_safe_json(evidence / "process-cleanup.json", cleanup, tokens)
        receiver_log.close()
        forgejo_log.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--repo-root", default=str(SCRIPT_DIR.parent))
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    args = parser.parse_args()
    try:
        result = run(args)
    except Exception as error:
        result = {"status": "failed", "error_type": type(error).__name__}
        print(json.dumps(result, sort_keys=True))
        return 1
    print(json.dumps({"status": result.get("status"), "run_id": result.get("run_identity", {}).get("id"), "event": result.get("native_webhook", {}).get("event"), "current_status": result.get("reconciliation", {}).get("current_status")}, sort_keys=True))
    return 0 if result.get("status") == "passed" and result.get("process_cleanup", {}).get("confirmed") else 1


if __name__ == "__main__":
    raise SystemExit(main())
