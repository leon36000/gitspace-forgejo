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
SELF_REPORT = EVIDENCE / "manifest-update.log"
SELF_REPORT_SHA = EVIDENCE / "manifest-update.log.sha256"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def local_files() -> list[Path]:
    return sorted(
        path
        for path in EVIDENCE.rglob("*")
        if path.is_file()
        and path.relative_to(EVIDENCE).as_posix()
        not in {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt", "manifest-update.log"}
    )


def parse_latest(manifest: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for line in manifest.read_text().splitlines():
        if len(line) >= 67 and line[64:66] == "  ":
            digest, relative = line[:64], line[66:]
            if all(ch in "0123456789abcdef" for ch in digest):
                latest[relative] = digest
    return latest


def append_current(manifest: Path, base: Path, relatives: set[Path]) -> int:
    original = manifest.read_bytes()
    latest = parse_latest(manifest)
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


def check_latest(manifest: Path, base: Path, relatives: set[Path]) -> list[str]:
    latest = parse_latest(manifest)
    failures = []
    for relative in sorted(relatives):
        path = base / relative
        if not path.is_file():
            failures.append(f"missing file: {relative}")
        elif latest.get(relative.as_posix()) != sha256(path):
            failures.append(f"latest hash missing or stale: {relative}")
    return failures


run_artifacts = local_files()
if SELF_REPORT.exists() or SELF_REPORT_SHA.exists():
    if not SELF_REPORT.is_file() or not SELF_REPORT_SHA.is_file():
        raise SystemExit("manifest self-report and its SHA-256 sidecar must be present together")
    if SELF_REPORT_SHA.read_text().strip() != sha256(SELF_REPORT):
        raise SystemExit("manifest self-report SHA-256 sidecar is stale")
LOCAL_MANIFEST.write_text(
    "".join(
        f"{sha256(path)}  {path.relative_to(EVIDENCE).as_posix()}\n"
        for path in run_artifacts
    )
)
local_failures = []
for line in LOCAL_MANIFEST.read_text().splitlines():
    digest, name = line.split("  ", 1)
    if sha256(EVIDENCE / name) != digest:
        local_failures.append(f"local SHA mismatch: {name}")
if local_failures:
    raise SystemExit("\n".join(local_failures))
LOCAL_CHECK.write_text(
    f"PASS: local SHA256SUMS verifies {len(run_artifacts)} run artifacts\n"
    + (
        "PASS: manifest-update.log is excluded to avoid self-reference and checked by its SHA-256 sidecar\n"
        if SELF_REPORT.exists()
        else "PASS: no self-report log is present in this evidence directory\n"
    )
)

core_paths = {
    Path("docs/architecture.md"),
    Path("docs/checkpoint.md"),
    Path("docs/refusal-evidence-matrix.md"),
    Path("docs/source-inventory.md"),
    Path("src/bin/forgejo-reconcile-local.rs"),
    Path("src/webhook.rs"),
    Path("src/webhook/reconcile.rs"),
    Path("tests/forgejo_reconcile_capabilities_e2e.rs"),
    Path("tests/webhook_handler_contract.rs"),
    Path("tests/webhook_mission_admission_contract.rs"),
    Path("tests/webhook_receiver_process_e2e.rs"),
    Path("tests/webhook_reconcile_contract.rs"),
    Path("tests/webhook_reconcile_pipeline.rs"),
    Path("scripts/run_webhook_reconcile_cli_e2e.py"),
}
run_paths = {
    path.relative_to(ROOT)
    for path in EVIDENCE.rglob("*")
    if path.is_file()
    and path.relative_to(EVIDENCE).as_posix()
    not in {"global-manifest-check.txt", "manifest-update.log"}
}
required = core_paths | run_paths
local_global = ROOT / "evidence/SHA256SUMS"
parent_global = PARENT / "evidence/SHA256SUMS"
local_added = append_current(local_global, ROOT, required)
parent_required = {PARENT_PREFIX / path for path in required}
parent_added = append_current(parent_global, PARENT, parent_required)

local_failures = check_latest(local_global, ROOT, required)
parent_failures = check_latest(parent_global, PARENT, parent_required)
if local_failures or parent_failures:
    raise SystemExit(
        "\n".join(
            [*(f"local: {failure}" for failure in local_failures), *(f"parent: {failure}" for failure in parent_failures)]
        )
    )
GLOBAL_CHECK.write_text(
    f"PASS: current append-only hashes match {len(required)} product paths\n"
    f"PASS: current parent append-only hashes match {len(parent_required)} paths\n"
    f"Appended changed hashes: product={local_added}, parent={parent_added}; prior prefixes preserved.\n"
)

check_path = GLOBAL_CHECK.relative_to(ROOT)
parent_check_path = PARENT_PREFIX / check_path
append_current(local_global, ROOT, {check_path})
append_current(parent_global, PARENT, {parent_check_path})
local_failures = check_latest(local_global, ROOT, required)
parent_failures = check_latest(parent_global, PARENT, parent_required)
if local_failures or parent_failures:
    raise SystemExit(
        "\n".join(
            [*(f"local: {failure}" for failure in local_failures), *(f"parent: {failure}" for failure in parent_failures)]
        )
    )
print(GLOBAL_CHECK.read_text(), end="")
