#!/usr/bin/env python3
"""Run the local-only composition inside Bwrap, within an outer host-created scope."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import systemd_process_scope as process_scope  # noqa: E402


BWRAP = Path("/usr/bin/bwrap")
ENTRY = Path(__file__).with_name("bwrap_entry.py")
OUTER_TIMEOUT_SECONDS = 1900
EXPECTED_FORGEJO_SHA256 = "e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7"


def sha256(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def safe_diagnostic(raw: bytes) -> str:
    value = raw.decode("utf-8", "replace")
    value = re.sub(
        r"(?i)(token|secret|credential|password|api[_-]?key)(\s*[=:]\s*)[^\s,;]+",
        r"\1\2[redacted]",
        value,
    )
    value = re.sub(r"(?<!sha256=)\b[A-Za-z0-9_-]{32,}\b", "[redacted-long-value]", value)
    return value[-2048:]


def main() -> int:
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--state-dir", required=True, type=Path)
    parser.add_argument("--forgejo-binary", required=True, type=Path)
    parser.add_argument("--evidence-dir", required=True, type=Path)
    parser.add_argument("--execution-id", required=True)
    parser.add_argument("--allocation-id", required=True)
    parser.add_argument("--budget-cap-usd", required=True)
    args = parser.parse_args()
    os.umask(0o077)
    host_network_namespace = os.readlink("/proc/self/ns/net")
    try:
        forgejo_sha256 = sha256(args.forgejo_binary.read_bytes())
    except OSError:
        print("refusing unavailable Forgejo test binary", file=sys.stderr)
        return 125
    try:
        evidence_root = (ROOT / "evidence").resolve(strict=True)
        evidence_parent = args.evidence_dir.parent.resolve(strict=True)
    except OSError:
        print("refusing unavailable evidence directory", file=sys.stderr)
        return 125
    if (
        re.fullmatch(r"net:\[[1-9][0-9]*\]", host_network_namespace) is None
        or args.state_dir.parent != Path("/tmp")
        or args.state_dir.exists()
        or args.state_dir.is_symlink()
        or evidence_parent != evidence_root
        or args.evidence_dir.exists()
        or args.evidence_dir.is_symlink()
        or args.forgejo_binary.is_symlink()
        or not args.forgejo_binary.is_file()
        or forgejo_sha256 != EXPECTED_FORGEJO_SHA256
        or args.budget_cap_usd != "0.20"
        or re.fullmatch(r"live-mcp-221", args.execution_id) is None
        or re.fullmatch(r"mission012-[A-Za-z0-9_-]{1,96}", args.allocation_id) is None
        or re.fullmatch(r"mission-012-live-agent-webhook-[a-z0-9-]{1,48}", args.evidence_dir.name) is None
    ):
        print("refusing invalid or non-private local composition inputs", file=sys.stderr)
        return 125
    if (
        not BWRAP.is_file()
        or BWRAP.is_symlink()
        or BWRAP.stat().st_uid != 0
        or BWRAP.stat().st_mode & 0o022
        or ENTRY.is_symlink()
        or not ENTRY.is_file()
    ):
        print("refusing untrusted Bwrap launcher or namespace entry", file=sys.stderr)
        return 125

    receipt_path = Path(__file__).with_name("launcher-bootstrap-live-mcp-221.json")
    if receipt_path.exists() or receipt_path.is_symlink():
        print("refusing to overwrite launcher bootstrap receipt", file=sys.stderr)
        return 125
    composition = [
        "/usr/bin/python3",
        str(ROOT / "scripts/run_mission_012_live_composed_e2e.py"),
        "--state-dir", str(args.state_dir),
        "--forgejo-binary", str(args.forgejo_binary),
        "--evidence-dir", str(args.evidence_dir),
        "--execution-id", args.execution_id,
        "--allocation-id", args.allocation_id,
        "--budget-cap-usd", args.budget_cap_usd,
        "--expected-host-network-namespace", host_network_namespace,
        "--agent-adapter", "local-test-adapter",
        "--sandbox-test-adapter",
        "--same-host-gate-barrier",
        "--reuse-attested-parent-scope",
    ]
    bwrap_command = [
        str(BWRAP),
        "--die-with-parent",
        "--unshare-net",
        "--ro-bind", "/", "/",
        "--dev-bind", "/dev", "/dev",
        "--proc", "/proc",
        "--bind", "/tmp", "/tmp",
        "--bind", str(ROOT), str(ROOT),
        "--chdir", str(ROOT),
        "--clearenv",
        "--setenv", "PATH", "/usr/bin:/bin",
        "--setenv", "HOME", str(Path.home()),
        "--setenv", "LANG", "C.UTF-8",
        "--setenv", "LC_ALL", "C.UTF-8",
        "--setenv", "TERM", "dumb",
        "--setenv", "NO_COLOR", "1",
        "--",
        "/usr/bin/python3",
        str(ENTRY),
        host_network_namespace,
        str(receipt_path),
        "--",
        *composition,
    ]
    env = {
        "PATH": "/usr/bin:/bin",
        "HOME": str(Path.home()),
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
    }
    try:
        outer = process_scope.prepare_process_scope(
            bwrap_command, env, OUTER_TIMEOUT_SECONDS
        )
    except process_scope.ProcessScopeUnavailable as error:
        print(f"FAIL-CLOSED: host scope preflight failed: {error}", file=sys.stderr)
        return 125
    started = process_scope.launch_scoped_process(
        outer,
        bwrap_command,
        env,
        cwd=ROOT,
        timeout=30,
        merge_stderr=False,
    )
    if started.process is None or started.cgroup_path is None:
        print("FAIL-CLOSED: host scope could not be attested before Bwrap entry", file=sys.stderr)
        return 125
    child = started.process
    child.stdin = None
    timed_out = False
    try:
        stdout, stderr = child.communicate(timeout=OUTER_TIMEOUT_SECONDS)
    except subprocess.TimeoutExpired:
        timed_out = True
        process_scope._abort_scope_process(child, outer)
        try:
            stdout, stderr = child.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            stdout, stderr = b"", b"outer scoped command did not exit after abort"
    cleanup = process_scope.finish_process_scope(outer, started.cgroup_path)
    summary = {
        "schema_version": 1,
        "host_network_namespace": host_network_namespace,
        "outer_scope_unit": outer.unit,
        "outer_scope_cgroup_path": started.cgroup_path,
        "scope_created_before_private_namespace": True,
        "private_bwrap_network_namespace_requested": True,
        "exit_code": child.returncode,
        "timed_out": timed_out,
        "scope_cleanup": cleanup,
        "stdout_bytes": len(stdout),
        "stdout_sha256": sha256(stdout),
        "stderr_bytes": len(stderr),
        "stderr_sha256": sha256(stderr),
        "raw_process_output_saved": False,
        "bootstrap_receipt_present": receipt_path.is_file() and not receipt_path.is_symlink(),
    }
    summary_path = Path(__file__).with_name("launcher-summary-live-mcp-221.json")
    if summary_path.exists() or summary_path.is_symlink():
        print("refusing to overwrite launcher summary", file=sys.stderr)
        return 125
    summary_path.write_text(json.dumps(summary, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    summary_path.chmod(0o600)
    if child.returncode != 0 or timed_out or cleanup != "clean":
        print(json.dumps(summary, sort_keys=True), file=sys.stderr)
        diagnostic = safe_diagnostic(stderr)
        if diagnostic:
            print(diagnostic, file=sys.stderr)
        return 1
    print(json.dumps(summary, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
