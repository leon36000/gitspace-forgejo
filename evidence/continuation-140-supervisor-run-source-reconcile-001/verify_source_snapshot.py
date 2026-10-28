#!/usr/bin/env python3
"""Compare MISSION-012 supervised 009's saved source hashes with this worktree."""
import hashlib
import json
from pathlib import Path

PRODUCT = Path(__file__).resolve().parents[2]
RUN = PRODUCT / "evidence/mission-012-live-agent-webhook-supervised-009/composed-run.json"
recorded = json.loads(RUN.read_text(encoding="utf-8"))["composition_source_sha256"]
matched, drifted, missing = [], [], []
current_hashes = {}
for name, old_hash in sorted(recorded.items()):
    path = PRODUCT / name
    if not path.is_file():
        missing.append(name)
        continue
    current_hash = hashlib.sha256(path.read_bytes()).hexdigest()
    current_hashes[name] = current_hash
    (matched if current_hash == old_hash else drifted).append(name)
print(json.dumps({
    "historical_run": "mission-012-live-agent-webhook-supervised-009",
    "historical_source_commit": "not established",
    "recorded_path_count": len(recorded),
    "matching_hash_count": len(matched),
    "drifted_hash_count": len(drifted),
    "missing_path_count": len(missing),
    "matching_paths": matched,
    "drifted_paths": drifted,
    "missing_paths": missing,
    "recorded_sha256": recorded,
    "current_sha256": current_hashes,
}, sort_keys=True, indent=2))
