#!/usr/bin/env python3
"""Reconcile CONT-245's Rust MCP gate receipt and source hashes with this tree."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUN = ROOT / "evidence/continuation-245-same-host-gate-001/successful-composition.log"
CORE = (
    "scripts/mission_012_mcp_supervisor.py",
    "scripts/mission_012_sandboxed_test_adapter.py",
    "scripts/mission_012_test_agent_adapter.py",
    "scripts/run_mission_012_live_mcp_agent_e2e.py",
    "scripts/run_mission_012_live_composed_e2e.py",
    "tests/test_mission_012_mcp_supervisor.py",
    "tests/test_mission_012_sandboxed_test_adapter.py",
    "tests/test_mission_012_test_agent_adapter.py",
    "src/mission_012_host.rs",
)


def sha256(path: Path) -> str:
    try:
        resolved = path.resolve(strict=True)
    except OSError:
        raise RuntimeError(f"source or receipt is unavailable: {path}") from None
    if (
        path.is_symlink()
        or not path.is_file()
        or not resolved.is_relative_to(ROOT.resolve(strict=True))
    ):
        raise RuntimeError(f"source or receipt is unavailable: {path}")
    return hashlib.sha256(path.read_bytes()).hexdigest()


if RUN.is_symlink() or not RUN.is_file():
    raise SystemExit("CONT-245 composition receipt is unavailable")
run_bytes = RUN.read_bytes()
run = json.loads(run_bytes)
snapshot = run.get("composition_source_sha256")
gate = run.get("same_host_gate_verification")
if not isinstance(snapshot, dict) or not isinstance(gate, dict):
    raise SystemExit("CONT-245 source snapshot or host-lifecycle receipt is missing")

matches: list[str] = []
mismatches: list[str] = []
for name, recorded in sorted(snapshot.items()):
    if not isinstance(name, str) or not isinstance(recorded, str):
        raise SystemExit("CONT-245 source snapshot has invalid entries")
    relative = Path(name)
    if relative.is_absolute() or not relative.parts or any(part in {".", ".."} for part in relative.parts):
        raise SystemExit("CONT-245 source snapshot contains a non-relative path")
    current = sha256(ROOT / relative)
    (matches if current == recorded else mismatches).append(name)

core_sources = {}
for name in CORE:
    recorded = snapshot.get(name)
    current = sha256(ROOT / name)
    core_sources[name] = {
        "recorded_sha256": recorded,
        "current_sha256": current,
        "matches": recorded == current,
    }

host = gate.get("mcp_host")
if not isinstance(host, dict):
    raise SystemExit("CONT-245 receipt has no MCP host binding")
core_matches = all(item["matches"] for item in core_sources.values())
receipt_valid = (
    gate.get("status") == "passed_while_admitted_mcp_host_remained_alive"
    and gate.get("observed_while_mcp_host_alive") is True
    and isinstance(host.get("pid"), int)
    and not isinstance(host.get("pid"), bool)
    and isinstance(host.get("start_time_ticks"), int)
    and not isinstance(host.get("start_time_ticks"), bool)
    and isinstance(host.get("executable_sha256"), str)
    and isinstance(gate.get("verifier"), dict)
    and gate["verifier"].get("exit_code") == 0
)
result = {
    "schema_version": 1,
    "audit_type": "read_only_source_and_receipt_reconciliation",
    "e2e_rerun": False,
    "receipt_path": RUN.relative_to(ROOT).as_posix(),
    "receipt_sha256": hashlib.sha256(run_bytes).hexdigest(),
    "receipt": {
        "status": gate.get("status"),
        "observed_while_mcp_host_alive": gate.get("observed_while_mcp_host_alive"),
        "host_binary_sha256": host.get("executable_sha256"),
        "independent_verifier_exit_code": gate["verifier"].get("exit_code"),
        "acceptance_summary": gate["verifier"].get("test_summary"),
        "lifecycle_limit": gate.get("lifecycle_limit"),
    },
    "source_snapshot": {
        "entries": len(snapshot),
        "matching_current_tree": len(matches),
        "differing_from_current_tree": len(mismatches),
        "full_snapshot_matches_current_tree": not mismatches,
        "differing_paths": mismatches,
    },
    "core_lifecycle_sources": core_sources,
    "core_lifecycle_sources_all_match": core_matches,
    "receipt_valid_for_core_lifecycle_claim": receipt_valid and core_matches,
}
print(json.dumps(result, sort_keys=True, indent=2))
if not receipt_valid or not core_matches or len(snapshot) != 114 or len(matches) != 109:
    raise SystemExit("source reconciliation did not match the expected archived evidence")
