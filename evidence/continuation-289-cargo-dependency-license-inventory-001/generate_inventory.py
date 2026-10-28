#!/usr/bin/env python3
"""Capture declared Cargo package license metadata from the locked offline graph."""
from __future__ import annotations

import collections
import csv
import datetime as dt
import hashlib
import json
import subprocess
from pathlib import Path

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[1]


def run(args: list[str]) -> str:
    result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True, check=True)
    return result.stdout.strip()


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

raw = run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"])
metadata = json.loads(raw)
packages = sorted(metadata["packages"], key=lambda item: (item["name"], item["version"], item["id"]))
workspace_ids = set(metadata["workspace_members"])

rows = []
expressions: collections.Counter[str] = collections.Counter()
source_kinds: collections.Counter[str] = collections.Counter()
undeclared = []
for package in packages:
    expression = package.get("license")
    license_file = package.get("license_file")
    raw_expression = expression if expression else ""
    if expression:
        expressions[expression] += 1
    elif license_file:
        expressions["LICENSE_FILE_DECLARED"] += 1
    else:
        expressions["UNDECLARED"] += 1
        undeclared.append({"name": package["name"], "version": package["version"], "source": package.get("source") or "path"})
    source = package.get("source") or "path"
    source_kinds["registry" if source.startswith("registry+") else "git" if source.startswith("git+") else "path"] += 1
    rows.append({
        "name": package["name"],
        "version": package["version"],
        "source": source,
        "license_expression_raw": raw_expression,
        "license_file_declared": "true" if license_file else "false",
        "workspace_member": "true" if package["id"] in workspace_ids else "false",
    })

with (OUT / "packages.csv").open("x", newline="", encoding="utf-8") as stream:
    writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
    writer.writeheader()
    writer.writerows(rows)

summary = {
    "captured_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
    "command": ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"],
    "cargo_version": run(["cargo", "--version"]),
    "rustc_version": run(["rustc", "--version"]),
    "cargo_lock_sha256": sha(ROOT / "Cargo.lock"),
    "cargo_toml_sha256": sha(ROOT / "Cargo.toml"),
    "package_count_in_metadata": len(packages),
    "workspace_member_count": len(workspace_ids),
    "source_kind_counts": dict(sorted(source_kinds.items())),
    "raw_declared_license_expression_counts": dict(sorted(expressions.items())),
    "packages_without_license_or_license_file_declaration": undeclared,
    "interpretation": "Cargo package metadata as exposed by the locked offline resolution; raw license strings are preserved verbatim and were not SPDX-validated. This is not a legal opinion, full license-text audit, compatibility determination, or redistribution authorization.",
}
with (OUT / "summary.json").open("x", encoding="utf-8") as stream:
    json.dump(summary, stream, ensure_ascii=False, indent=2, sort_keys=True)
    stream.write("\n")
print(json.dumps({"package_count": len(packages), "source_kind_counts": summary["source_kind_counts"], "raw_declared_license_expression_counts": summary["raw_declared_license_expression_counts"], "undeclared": undeclared, "cargo_lock_sha256": summary["cargo_lock_sha256"]}, ensure_ascii=False, sort_keys=True))
