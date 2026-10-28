from __future__ import annotations

import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(__file__).resolve().parent
PARENT = ROOT.parents[1]
PARENT_PREFIX = ROOT.relative_to(PARENT)
LOCAL_MANIFEST = EVIDENCE / "SHA256SUMS"
LOCAL_CHECK = EVIDENCE / "manifest-check.txt"
GLOBAL_CHECK = EVIDENCE / "global-manifest-check.txt"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def files_in_evidence() -> list[Path]:
    excluded = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
    return sorted(path for path in EVIDENCE.iterdir() if path.is_file() and path.name not in excluded)


def latest_hashes(manifest: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for line in manifest.read_text().splitlines():
        if len(line) >= 67 and line[64:66] == "  ":
            digest, relative = line[:64], line[66:]
            if all(char in "0123456789abcdef" for char in digest):
                latest[relative] = digest
    return latest


def append_current(manifest: Path, base: Path, relatives: set[Path]) -> int:
    original = manifest.read_bytes()
    latest = latest_hashes(manifest)
    additions = []
    for relative in sorted(relatives):
        path = base / relative
        if not path.is_file():
            raise FileNotFoundError(path)
        digest = sha256(path)
        name = relative.as_posix()
        if latest.get(name) != digest:
            additions.append(f"{digest}  {name}\n")
    if additions:
        with manifest.open("ab") as stream:
            if original and not original.endswith(b"\n"):
                stream.write(b"\n")
            stream.writelines(line.encode() for line in additions)
    if not manifest.read_bytes().startswith(original):
        raise RuntimeError(f"append-only prefix changed: {manifest}")
    return len(additions)


def check_current(manifest: Path, base: Path, relatives: set[Path]) -> list[str]:
    latest = latest_hashes(manifest)
    failures = []
    for relative in sorted(relatives):
        path = base / relative
        if not path.is_file():
            failures.append(f"missing file: {relative}")
        elif latest.get(relative.as_posix()) != sha256(path):
            failures.append(f"latest hash missing or stale: {relative}")
    return failures


run_artifacts = files_in_evidence()
LOCAL_MANIFEST.write_text("".join(f"{sha256(path)}  {path.name}\n" for path in run_artifacts))
local_failures = []
for line in LOCAL_MANIFEST.read_text().splitlines():
    digest, name = line.split("  ", 1)
    if sha256(EVIDENCE / name) != digest:
        local_failures.append(f"local SHA mismatch: {name}")
if local_failures:
    raise SystemExit("\n".join(local_failures))
LOCAL_CHECK.write_text(f"PASS: local SHA256SUMS verifies {len(run_artifacts)} run artifacts\n")

core_paths = {
    Path("docs/checkpoint.md"),
    Path("docs/implementation-plan.md"),
    Path("tests/webhook_receiver_process_e2e.rs"),
    Path("scripts/run_webhook_reconcile_cli_e2e.py"),
}
run_paths = {
    EVIDENCE.relative_to(ROOT) / path.name
    for path in EVIDENCE.iterdir()
    if path.is_file() and path.name != "global-manifest-check.txt"
}
required = core_paths | run_paths
product_manifest = ROOT / "evidence/SHA256SUMS"
parent_manifest = PARENT / "evidence/SHA256SUMS"
product_added = append_current(product_manifest, ROOT, required)
parent_required = {PARENT_PREFIX / path for path in required}
parent_added = append_current(parent_manifest, PARENT, parent_required)

product_failures = check_current(product_manifest, ROOT, required)
parent_failures = check_current(parent_manifest, PARENT, parent_required)
if product_failures or parent_failures:
    raise SystemExit("\n".join(
        [*(f"product: {failure}" for failure in product_failures),
         *(f"parent: {failure}" for failure in parent_failures)]
    ))
GLOBAL_CHECK.write_text(
    f"PASS: current append-only hashes match {len(required)} product paths\n"
    f"PASS: current parent append-only hashes match {len(parent_required)} paths\n"
    f"Appended changed hashes: product={product_added}, parent={parent_added}; prior prefixes preserved.\n"
)

global_check = GLOBAL_CHECK.relative_to(ROOT)
parent_global_check = PARENT_PREFIX / global_check
append_current(product_manifest, ROOT, {global_check})
append_current(parent_manifest, PARENT, {parent_global_check})
product_failures = check_current(product_manifest, ROOT, required)
parent_failures = check_current(parent_manifest, PARENT, parent_required)
if product_failures or parent_failures:
    raise SystemExit("\n".join(
        [*(f"product: {failure}" for failure in product_failures),
         *(f"parent: {failure}" for failure in parent_failures)]
    ))
print(GLOBAL_CHECK.read_text(), end="")
