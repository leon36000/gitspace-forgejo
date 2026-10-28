#!/usr/bin/env python3
"""Summarize the exact resolved registry graph and local license-file hashes."""
from __future__ import annotations

import hashlib
import json
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(__file__).resolve().parent
METADATA = json.loads((EVIDENCE / "cargo-metadata.json").read_text(encoding="utf-8"))


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


packages = [
    package
    for package in METADATA["packages"]
    if (package.get("source") or "").startswith("registry+")
]
root_package = next(package for package in METADATA["packages"] if package["name"] == "agent-forge-core")
direct_names = sorted(
    dependency["name"]
    for dependency in root_package["dependencies"]
    if (dependency.get("source") or "").startswith("registry+")
    and dependency.get("kind") is None
)
records = []
license_expressions: Counter[str] = Counter()
for package in sorted(packages, key=lambda item: (item["name"], item["version"])):
    crate_root = Path(package["manifest_path"]).parent
    conventional = []
    if crate_root.is_dir() and not crate_root.is_symlink():
        for path in sorted(crate_root.iterdir()):
            if path.is_file() and not path.is_symlink() and path.name.upper().startswith(
                ("LICENSE", "LICENCE", "COPYING", "NOTICE")
            ):
                conventional.append(
                    {"name": path.name, "size": path.stat().st_size, "sha256": digest(path)}
                )
    expression = package.get("license")
    if expression:
        license_expressions[expression] += 1
    records.append(
        {
            "name": package["name"],
            "version": package["version"],
            "source": package["source"],
            "license": expression,
            "license_file": package.get("license_file"),
            "conventional_license_files_in_local_cache": conventional,
        }
    )

report = {
    "cargo_lock_sha256": digest(ROOT / "Cargo.lock"),
    "external_registry_package_count": len(records),
    "direct_registry_dependency_count": len(direct_names),
    "direct_registry_dependencies": direct_names,
    "packages_missing_license_metadata": [
        f"{item['name']}@{item['version']}"
        for item in records
        if not item["license"] and not item["license_file"]
    ],
    "packages_with_at_least_one_conventional_license_file": sum(
        bool(item["conventional_license_files_in_local_cache"]) for item in records
    ),
    "raw_license_expression_counts": dict(sorted(license_expressions.items())),
    "packages": records,
    "product_cargo_license_state": {
        "license": root_package["license"],
        "license_file": root_package["license_file"],
        "publish": root_package["publish"],
        "decision": root_package["metadata"]["agent-forge"]["licensing"]["decision"],
        "redistribution_rights": root_package["metadata"]["agent-forge"]["licensing"][
            "redistribution_rights"
        ],
        "spdx_package_license_declared": root_package["metadata"]["agent-forge"]["licensing"][
            "spdx_package_license_declared"
        ],
    },
}
(EVIDENCE / "dependency-license-inventory.json").write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
print(
    json.dumps(
        {
            "external_registry_package_count": report["external_registry_package_count"],
            "direct_registry_dependency_count": report["direct_registry_dependency_count"],
            "packages_missing_license_metadata": len(report["packages_missing_license_metadata"]),
            "packages_with_local_license_file": report[
                "packages_with_at_least_one_conventional_license_file"
            ],
            "cargo_lock_sha256": report["cargo_lock_sha256"],
        },
        sort_keys=True,
    )
)
