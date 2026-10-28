from __future__ import annotations

import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(__file__).resolve().parent
PARENT = ROOT.parents[1]
PARENT_PREFIX = ROOT.relative_to(PARENT)

local_required = {
    Path("docs/architecture.md"),
    Path("docs/source-inventory.md"),
    Path("docs/checkpoint.md"),
    Path("Cargo.toml"),
    Path("Cargo.lock"),
    Path("src/bin/forgejo-reconcile-local.rs"),
    Path("scripts/verify_cargo_license_state.py"),
    Path("scripts/test_cargo_license_state.py"),
    EVIDENCE.relative_to(ROOT) / "SHA256SUMS",
}
local_required.update(
    EVIDENCE.relative_to(ROOT) / path.name
    for path in EVIDENCE.iterdir()
    if path.is_file()
    and path.name not in {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
)
local_required.add(EVIDENCE.relative_to(ROOT) / "manifest-check.txt")


def latest_hashes(manifest: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for line in manifest.read_text().splitlines():
        if len(line) >= 67 and line[64:66] == "  ":
            digest, path = line[:64], line[66:]
            if all(character in "0123456789abcdef" for character in digest):
                latest[path] = digest
    return latest


def check(manifest: Path, root: Path, required: set[Path]) -> list[str]:
    latest = latest_hashes(manifest)
    failures = []
    for relative in sorted(required):
        path = root / relative
        if not path.is_file():
            failures.append(f"missing file: {relative}")
            continue
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if latest.get(relative.as_posix()) != actual:
            failures.append(f"latest hash missing or stale: {relative}")
    return failures


local_manifest = ROOT / "evidence/SHA256SUMS"
local_failures = check(local_manifest, ROOT, local_required)
parent_required = {PARENT_PREFIX / relative for relative in local_required}
parent_manifest = PARENT / "evidence/SHA256SUMS"
parent_failures = check(parent_manifest, PARENT, parent_required)

if local_failures or parent_failures:
    for failure in local_failures:
        print(f"local: {failure}")
    for failure in parent_failures:
        print(f"parent: {failure}")
    raise SystemExit(1)

print(f"PASS: current local hashes match {len(local_required)} paths")
print(f"PASS: current parent hashes match {len(parent_required)} paths")
