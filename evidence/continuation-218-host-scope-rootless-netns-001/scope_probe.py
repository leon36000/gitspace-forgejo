#!/usr/bin/env python3
"""Check a host-attested user scope that enters a rootless private network namespace."""

from __future__ import annotations

import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import systemd_process_scope as process_scope  # noqa: E402


def main() -> int:
    host_netns = os.readlink("/proc/self/ns/net")
    child_code = r'''import json, os, stat, sys
from pathlib import Path
sys.path.insert(0, str(Path.cwd() / "scripts"))
import systemd_process_scope as scope
refusal = None
try:
    scope.prepare_process_scope(
        ["/usr/bin/true"],
        {"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
        2,
    )
except scope.ProcessScopeUnavailable as error:
    refusal = str(error)
files = {
    str(path): {
        "uid_seen_in_user_namespace": path.lstat().st_uid,
        "mode": oct(stat.S_IMODE(path.lstat().st_mode)),
        "trusted_by_existing_helper": scope._trusted_systemd_binary(path),
    }
    for path in (scope.SYSTEMD_RUN, scope.SYSTEMCTL)
}
print(json.dumps({
    "euid_in_user_namespace": os.geteuid(),
    "network_namespace": os.readlink("/proc/self/ns/net"),
    "uid_map": Path("/proc/self/uid_map").read_text().strip(),
    "gid_map": Path("/proc/self/gid_map").read_text().strip(),
    "cgroup": next(line[3:] for line in Path("/proc/self/cgroup").read_text().splitlines() if line.startswith("0::/")),
    "systemd_binaries": files,
    "nested_scope_refusal": refusal,
    "nested_command_started": False,
}, sort_keys=True))'''
    environment = {
        "PATH": "/usr/bin:/bin",
        "HOME": "/nonexistent",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
    }
    command = [
        "/usr/bin/unshare", "--user", "--map-root-user", "--net",
        "/usr/bin/python3", "-c", child_code,
    ]
    scope = process_scope.prepare_process_scope(command, environment, 10)
    started = process_scope.launch_scoped_process(
        scope,
        command,
        environment,
        cwd=ROOT,
        timeout=10,
        merge_stderr=True,
    )
    if started.process is None or started.cgroup_path is None:
        raise RuntimeError("trusted host scope could not attest its waiting child")

    process = started.process
    try:
        # launch_scoped_process closes this channel after its nonce-bound handshake.
        process.stdin = None
        output, _ = process.communicate(timeout=10)
    except BaseException:
        process_scope._abort_scope_process(process, scope)
        raise

    cleanup = process_scope.finish_process_scope(scope, started.cgroup_path)
    if process.returncode != 0 or len(output) > 64 * 1024:
        raise RuntimeError("scoped namespace probe exited unsuccessfully or exceeded output limit")
    child = json.loads(output.decode("utf-8"))
    observed = {
        "host_network_namespace": host_netns,
        "scope_unit": scope.unit,
        "attested_cgroup_path": started.cgroup_path,
        "scope_attested_before_command": True,
        "child_exit_code": process.returncode,
        "cleanup": cleanup,
        "child": child,
    }
    if child["network_namespace"] == host_netns:
        raise RuntimeError("rootless child inherited the host network namespace")
    if child["cgroup"] != started.cgroup_path:
        raise RuntimeError("rootless child escaped the host-attested cgroup")
    if child["nested_scope_refusal"] != "trusted systemd-run/systemctl binaries are unavailable":
        raise RuntimeError("inner scope preflight did not fail at the expected trust check")
    if any(info["trusted_by_existing_helper"] for info in child["systemd_binaries"].values()):
        raise RuntimeError("existing helper unexpectedly trusted unmapped systemd ownership")
    if cleanup != "clean":
        raise RuntimeError(f"transient scope cleanup was not clean: {cleanup}")
    print(json.dumps(observed, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
