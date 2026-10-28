#!/usr/bin/env python3
"""Summarize isolated probe dependency closures from Cargo metadata."""
from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
metadata = json.loads((ROOT / "cargo-metadata.json").read_text(encoding="utf-8"))
packages = {item["id"]: item for item in metadata["packages"]}
nodes = {item["id"]: item for item in metadata["resolve"]["nodes"]}
root_id = metadata["resolve"]["root"]
root_node = nodes[root_id]
direct = {item["name"]: item["pkg"] for item in root_node["deps"]}


def closure(package_id: str) -> list[dict[str, str | None]]:
    pending = [package_id]
    seen: set[str] = set()
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        pending.extend(item["pkg"] for item in nodes[current]["deps"])
    rows = []
    for current in sorted(seen, key=lambda key: (packages[key]["name"], packages[key]["version"])):
        package = packages[current]
        rows.append({
            "name": package["name"],
            "version": package["version"],
            "license": package.get("license"),
        })
    return rows


summary = {"candidates": {}}
for metadata_name, label in (("ed25519_dalek", "ed25519-dalek"), ("ring", "ring")):
    package_id = direct[metadata_name]
    rows = closure(package_id)
    summary["candidates"][label] = {
        "version": packages[package_id]["version"],
        "license": packages[package_id].get("license"),
        "dependency_count_including_candidate": len(rows),
        "missing_license_count": sum(row["license"] is None for row in rows),
        "packages": rows,
    }

(ROOT / "candidate-closures.json").write_text(
    json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
