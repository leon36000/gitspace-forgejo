#!/usr/bin/env python3
"""Emit only the two non-secret Forgejo hook-task acknowledgment fields."""

import json
from pathlib import Path
import sys


WORKTREE = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKTREE / "scripts"))
import run_forgejo_live_webhook_native_replay_e2e as replay  # noqa: E402


DATABASE = Path("/tmp/forgejo-team-review-cont212-state-002/data/forgejo.db")
if not DATABASE.is_file() or DATABASE.is_symlink():
    raise SystemExit("retained local Forgejo test database is missing or unsafe")
rows = replay.hook_task_readback(DATABASE, 1)
summary = {
    "source": "read-only Forgejo SQLite hook_task projection; raw payloads omitted",
    "rows": [
        {
            key: row.get(key)
            for key in (
                "task_id",
                "hook_id",
                "uuid",
                "event_type",
                "is_delivered",
                "is_succeed",
                "response_status",
            )
        }
        for row in rows
    ],
}
print(json.dumps(summary, indent=2, sort_keys=True))
