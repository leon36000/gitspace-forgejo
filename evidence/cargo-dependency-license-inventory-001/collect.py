#!/usr/bin/env python3
"""Snapshot locked Cargo license metadata and hashes of cached license texts."""
from __future__ import annotations
import hashlib
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LICENSE_NAME = re.compile(r"(?i)^(license|licence|copying|notice)([-_.].*)?$")

def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main() -> int:
    completed = subprocess.run(
        ["cargo", "metadata", "--offline", "--locked", "--format-version", "1"],
        cwd=ROOT, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    metadata = json.loads(completed.stdout)
    workspace = set(metadata["workspace_members"])
    root_package = next(pkg for pkg in metadata["packages"] if pkg["id"] in workspace)
    root_node = next(node for node in metadata["resolve"]["nodes"] if node["id"] == root_package["id"])
    direct_ids = {dep["pkg"] for dep in root_node["deps"]}
    lock_data = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    registry_packages = []
    missing_metadata = []
    missing_text = []
    for pkg in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"], item["id"])):
        source = pkg.get("source")
        if not isinstance(source, str) or not source.startswith("registry+"):
            continue
        manifest = Path(pkg["manifest_path"])
        package_root = manifest.parent
        found = []
        for item in package_root.iterdir():
            if item.name == "Cargo.toml" or not LICENSE_NAME.fullmatch(item.name):
                continue
            st = item.lstat()
            if item.is_file() and not item.is_symlink():
                found.append({"name": item.name, "size_bytes": st.st_size, "sha256": digest(item)})
        declared_file = pkg.get("license_file")
        if declared_file:
            target = package_root / declared_file
            if target.is_file() and not target.is_symlink() and all(entry["name"] != target.name for entry in found):
                st = target.stat()
                found.append({"name": target.name, "size_bytes": st.st_size, "sha256": digest(target), "declared_license_file": True})
        found.sort(key=lambda entry: entry["name"])
        if not pkg.get("license"):
            missing_metadata.append(f"{pkg['name']} {pkg['version']}")
        if not found:
            missing_text.append(f"{pkg['name']} {pkg['version']}")
        registry_packages.append({
            "name": pkg["name"],
            "version": pkg["version"],
            "cargo_license_expression_raw": pkg.get("license"),
            "cargo_license_file_raw": declared_file,
            "direct_dependency": pkg["id"] in direct_ids,
            "source": "crates.io registry source in local Cargo cache",
            "manifest_sha256": digest(manifest),
            "license_files": found,
        })
    output = {
        "schema_version": 1,
        "cargo_metadata_command": "cargo metadata --offline --locked --format-version 1",
        "cargo_lock_sha256": digest(ROOT / "Cargo.lock"),
        "workspace_package": {
            "name": root_package["name"],
            "version": root_package["version"],
            "license": root_package.get("license"),
            "license_file": root_package.get("license_file"),
            "publish": root_package.get("publish"),
            "license_decision": root_package.get("metadata", {}).get("agent-forge", {}).get("licensing", {}).get("decision"),
            "redistribution_rights": root_package.get("metadata", {}).get("agent-forge", {}).get("licensing", {}).get("redistribution_rights"),
        },
        "locked_registry_package_count": len(registry_packages),
        "direct_registry_dependency_count": sum(1 for pkg in registry_packages if pkg["direct_dependency"]),
        "missing_license_metadata": missing_metadata,
        "missing_conventional_license_texts": missing_text,
        "registry_packages": registry_packages,
        "interpretation": "Raw Cargo metadata and local license-file hashes only; no legal compatibility or redistribution decision.",
    }
    json.dump(output, sys.stdout, ensure_ascii=False, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0 if not missing_metadata and not missing_text else 2

if __name__ == "__main__":
    raise SystemExit(main())
