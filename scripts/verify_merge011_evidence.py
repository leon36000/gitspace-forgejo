#!/usr/bin/env python3
"""Check the saved MERGE-011 JSON and rerun its final offline classifier."""

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from run_forgejo_base_freshness_race_e2e import classify_race_result


EVIDENCE = ROOT / "evidence"
JSON_FILES = [
    EVIDENCE / "merge-011-base-freshness-race-source-toctou.json",
    *(EVIDENCE / f"merge-011-base-freshness-race-{run}.json" for run in ("008", "009", "010", "011", "012")),
    *(EVIDENCE / f"merge-011-base-freshness-race-{run}-premerge-summary.json" for run in ("009", "010", "011", "012")),
]


def main() -> int:
    parsed = {path.name: json.loads(path.read_text()) for path in JSON_FILES}
    source = parsed["merge-011-base-freshness-race-source-toctou.json"]
    if source.get("classification") != "toctou_reproduced":
        raise SystemExit("source-level evidence classification changed unexpectedly")
    for run in ("009", "010", "011", "012"):
        name = f"merge-011-base-freshness-race-{run}.json"
        actual = classify_race_result(parsed[name])
        if actual != "stale_base_merge_refused":
            raise SystemExit(f"{run}: expected stale_base_merge_refused, got {actual}")
        print(f"{run}: {actual}")
    print(f"{len(parsed)} MERGE-011 JSON files parse successfully")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
