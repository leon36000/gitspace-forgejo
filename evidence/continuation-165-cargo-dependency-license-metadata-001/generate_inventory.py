#!/usr/bin/env python3
"""Read the locked Cargo dependency graph offline and export declared metadata."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[2]
OUTPUT = Path(__file__).resolve().parent
PACKAGE_NAME = "agent-forge-core"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


completed = subprocess.run(
    ["cargo", "metadata", "--offline", "--locked", "--format-version", "1", "--all-features"],
    cwd=ROOT,
    check=True,
    capture_output=True,
    text=True,
)
metadata = json.loads(completed.stdout)
root_packages = [package for package in metadata["packages"] if package["name"] == PACKAGE_NAME]
if len(root_packages) != 1:
    raise SystemExit("expected exactly one product package in Cargo metadata")
root = root_packages[0]
resolve = metadata.get("resolve")
if not isinstance(resolve, dict):
    raise SystemExit("Cargo metadata omitted the resolved dependency graph")
nodes = {node["id"]: node for node in resolve.get("nodes", [])}
if root["id"] not in nodes:
    raise SystemExit("Cargo metadata omitted the product package node")

direct_kinds: dict[str, set[str]] = {}
for dependency in nodes[root["id"]].get("deps", []):
    kinds = dependency.get("dep_kinds") or [{}]
    direct_kinds[dependency["pkg"]] = {
        str(item.get("kind") or "normal") for item in kinds
    }

reachable = {root["id"]}
pending = [root["id"]]
while pending:
    current = pending.pop()
    for dependency in nodes.get(current, {}).get("deps", []):
        package_id = dependency["pkg"]
        if package_id not in reachable:
            reachable.add(package_id)
            pending.append(package_id)

packages_by_id = {package["id"]: package for package in metadata["packages"]}
if reachable - packages_by_id.keys():
    raise SystemExit("Cargo metadata graph contains unresolved package IDs")
external = sorted(
    (packages_by_id[package_id] for package_id in reachable if package_id != root["id"]),
    key=lambda package: (package["name"], package["version"], package.get("source") or ""),
)
rows = [
    {
        "dependency_kind": (
            ",".join(sorted(direct_kinds[package["id"]]))
            if package["id"] in direct_kinds
            else "transitive"
        ),
        "name": package["name"],
        "version": package["version"],
        "license": package.get("license"),
        "license_file": package.get("license_file"),
        "source": package.get("source") or "path",
    }
    for package in external
]
missing = [row for row in rows if not row["license"] and not row["license_file"]]
if missing:
    raise SystemExit(
        "one or more resolved dependencies lack declared license metadata: "
        + ", ".join(f"{row['name']}@{row['version']}" for row in missing)
    )

summary = {
    "schema_version": 1,
    "command": "cargo metadata --offline --locked --format-version 1 --all-features",
    "root_package": {
        "name": root["name"],
        "version": root["version"],
        "license": root.get("license"),
        "license_file": root.get("license_file"),
        "publish": root.get("publish"),
    },
    "cargo_lock_sha256": sha256(ROOT / "Cargo.lock"),
    "resolved_external_packages": len(rows),
    "direct_dependency_packages": sum(row["dependency_kind"] != "transitive" for row in rows),
    "missing_external_license_metadata": len(missing),
    "packages": rows,
    "interpretation": "Cargo package-declared metadata only; not a source, license compatibility, notice, patent, or redistribution rights review.",
}
(OUTPUT / "dependency-license-metadata.json").write_text(
    json.dumps(summary, sort_keys=True, indent=2) + "\n", encoding="utf-8"
)
with (OUTPUT / "dependency-licenses.tsv").open("w", encoding="utf-8", newline="") as stream:
    stream.write("dependency_kind\tname\tversion\tlicense\tlicense_file\tsource\n")
    for row in rows:
        stream.write(
            "\t".join(
                str(row[field] or "")
                for field in (
                    "dependency_kind",
                    "name",
                    "version",
                    "license",
                    "license_file",
                    "source",
                )
            )
            + "\n"
        )
print(
    json.dumps(
        {
            "resolved_external_packages": len(rows),
            "direct_dependency_packages": summary["direct_dependency_packages"],
            "missing_external_license_metadata": len(missing),
            "product_license_selected": root.get("license") is not None
            or root.get("license_file") is not None,
            "cargo_lock_sha256": summary["cargo_lock_sha256"],
        },
        sort_keys=True,
    )
)
