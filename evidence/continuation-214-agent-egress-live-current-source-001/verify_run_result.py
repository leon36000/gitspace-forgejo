#!/usr/bin/env python3
"""Independently validate the current-source local egress receipt and bindings."""
from __future__ import annotations
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / 'scripts'))
spec = importlib.util.spec_from_file_location('egress_runner', ROOT / 'scripts/run_forgejo_agent_egress_live_e2e.py')
if spec is None or spec.loader is None:
    raise SystemExit('runner could not be imported')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
result = json.loads((EVIDENCE / 'result.json').read_text(encoding='utf-8'))
provenance = json.loads((EVIDENCE / 'provenance.json').read_text(encoding='utf-8'))
runner.validate_live_egress_result(result)
checks = {
    'head': (EVIDENCE / 'git-head.txt').read_text(encoding='utf-8').strip() == provenance.get('head_commit'),
    'driver': hashlib.sha256((ROOT / 'scripts/run_forgejo_agent_egress_live_e2e.py').read_bytes()).hexdigest() == provenance.get('driver_sha256'),
    'adapter_copy': hashlib.sha256((EVIDENCE / 'sandbox-probe.py').read_bytes()).hexdigest() == provenance.get('adapter_probe_sha256'),
    'forgejo': result.get('forgejo', {}).get('binary_sha256') == provenance.get('forgejo_binary_sha256'),
    'all_routes_blocked': all(item.get('sandbox_blocked') is True for item in result.get('routes', [])),
    'canary_not_visible': result.get('credential_visible') is False,
    'trusted_read_only_control': result.get('trusted_read_control', {}).get('credential_passed_to_sandbox') is False,
}
if not all(checks.values()):
    raise SystemExit(json.dumps(checks, sort_keys=True))
print(json.dumps({'verified': True, 'checks': checks, 'route_count': len(result['routes']), 'head': provenance['head_commit']}, sort_keys=True))
