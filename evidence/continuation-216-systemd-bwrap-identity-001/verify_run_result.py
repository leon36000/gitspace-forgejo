#!/usr/bin/env python3
"""Check the bounded observations captured for CONTINUATION-216."""

import json
import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent
result = json.loads((ROOT / "results.json").read_text(encoding="utf-8"))
expected = {
    "schema_version": 1,
    "host_prepare": "ok",
    "adapter_bubblewrap": {
        "systemd_run_uid": 65534,
        "systemctl_uid": 65534,
        "systemd_run_trusted": False,
        "systemctl_trusted": False,
        "prepare": "trusted systemd-run/systemctl binaries are unavailable",
    },
}
for key, value in expected.items():
    if result.get(key) != value:
        raise SystemExit(f"CONTINUATION-216 result mismatch: {key}")
for field in ("gate_or_candidate_started", "forgejo_started", "llm_called", "external_service_called"):
    if result.get(field) is not False:
        raise SystemExit(f"CONTINUATION-216 unexpected activity: {field}")
outer = result.get("historical_outer_bubblewrap")
if not isinstance(outer, dict) or outer.get("bus_socket_visible") is not True or outer.get("dbus_contact_attempted") is not False:
    raise SystemExit("CONTINUATION-216 outer Bubblewrap observation mismatch")
service = result.get("user_systemd_private_network_probe")
if not isinstance(service, dict) or service.get("same_as_host_namespace") is not True or service.get("exit_code") != 0:
    raise SystemExit("CONTINUATION-216 transient service observation mismatch")
if result.get("unshare_net", {}).get("diagnostic") != "Operation not permitted":
    raise SystemExit("CONTINUATION-216 unshare refusal mismatch")
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, separator, relative = line.partition("  ")
    source = ROOT.parents[1] / relative
    if not separator or hashlib.sha256(source.read_bytes()).hexdigest() != value:
        raise SystemExit(f"CONTINUATION-216 source hash mismatch: {relative}")
print("CONTINUATION-216 bounded result verified")
