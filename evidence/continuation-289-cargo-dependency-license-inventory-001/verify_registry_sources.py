#!/usr/bin/env python3
"""Verify locked registry archive checksums and hash package license documents."""
from __future__ import annotations

import hashlib
import json
import subprocess
import tomllib
from pathlib import Path

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[1]


def run_json(args: list[str]) -> dict:
    result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True, check=True)
    return json.loads(result.stdout)


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
metadata = run_json(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"])
lock_packages = {(p["name"], p["version"]): p for p in lock["package"]}
cache = Path.home() / ".cargo" / "registry" / "cache"
rows = []
for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"], p["id"])):
    source = package.get("source") or "path"
    if not source.startswith("registry+"):
        continue
    key = (package["name"], package["version"])
    lock_entry = lock_packages.get(key)
    if not lock_entry or not lock_entry.get("checksum"):
        raise SystemExit(f"missing Cargo.lock checksum for {key}")
    archive_candidates = sorted(cache.glob(f"*/{package['name']}-{package['version']}.crate"))
    if len(archive_candidates) != 1:
        raise SystemExit(f"expected one cached archive for {key}, found {len(archive_candidates)}")
    archive_hash = sha(archive_candidates[0])
    expected_hash = lock_entry["checksum"]
    if archive_hash != expected_hash:
        raise SystemExit(f"Cargo.lock checksum mismatch for {key}")
    package_root = Path(package["manifest_path"]).parent
    docs = []
    for path in sorted(package_root.iterdir(), key=lambda item: item.name.casefold()):
        if not path.is_file():
            continue
        name = path.name.upper()
        if name.startswith(("LICENSE", "LICENCE", "COPYING")) or name == "UNLICENSE":
            docs.append({"file": path.name, "sha256": sha(path)})
    if not docs:
        raise SystemExit(f"no recognized top-level license document for {key}")
    rows.append({
        "name": package["name"],
        "version": package["version"],
        "cargo_source": source,
        "cargo_lock_checksum": expected_hash,
        "cached_archive_sha256": archive_hash,
        "checksum_matches": True,
        "license_expression_raw": package.get("license"),
        "license_file_declared": bool(package.get("license_file")),
        "top_level_license_documents": docs,
    })

result = {
    "command": ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"],
    "registry_package_count": len(rows),
    "registry_archives_checksum_verified": sum(row["checksum_matches"] for row in rows),
    "registry_packages_with_recognized_top_level_license_document": sum(bool(row["top_level_license_documents"]) for row in rows),
    "scope_note": "Checksums match the locked cached .crate archives. The listed document hashes are from Cargo's extracted registry source trees. This verifies source correspondence/integrity for this local cache; it does not decide legal compatibility, license interpretation, notice obligations, or redistribution rights.",
    "packages": rows,
}
with (OUT / "registry-source-provenance.json").open("x", encoding="utf-8") as stream:
    json.dump(result, stream, ensure_ascii=False, indent=2, sort_keys=True)
    stream.write("\n")
print(json.dumps({"registry_package_count": len(rows), "archives_checksum_verified": result["registry_archives_checksum_verified"], "packages_with_license_docs": result["registry_packages_with_recognized_top_level_license_document"]}, sort_keys=True))
