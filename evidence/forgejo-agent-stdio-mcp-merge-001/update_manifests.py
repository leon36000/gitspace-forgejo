from __future__ import annotations

import hashlib
from pathlib import Path

HERE = Path(__file__).resolve()
ROOT = HERE.parents[2]
PARENT = ROOT.parents[1]
PREFIX = ROOT.relative_to(PARENT)
RUNS = [
    ROOT / "evidence" / "forgejo-agent-stdio-mcp-merge-001",
    ROOT / "evidence" / "forgejo-stdio-mcp-same-candidate-001",
    ROOT / "evidence" / "forgejo-stdio-mcp-same-candidate-webhook-001",
    ROOT / "evidence" / "forgejo-stdio-mcp-webhook-idempotent-replay-001",
    ROOT / "evidence" / "semantic-clean-merge-canary-001",
    ROOT / "evidence" / "forgejo-stdio-mcp-webhook-restart-replay-001",
    ROOT / "evidence" / "forgejo-semantic-clean-merge-live-001",
    ROOT / "evidence" / "cargo-license-state-unselected-008",
    ROOT / "evidence" / "webhook-capabilities-content-length-001",
]
LOCAL_EXCLUDED = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
SOURCE_PATHS = {
    Path("Cargo.lock"),
    Path("docs/source-inventory.md"),
    Path("scripts/run_forgejo_live_pr_merge_e2e.py"),
    Path("scripts/resume_forgejo_semantic_clean_merge.py"),
    Path("scripts/semantic_merge_canary.py"),
    Path("scripts/run_mission_012_stdio_mcp_egress_gate_e2e.py"),
    Path("scripts/mission_012_test_agent_adapter.py"),
    Path("scripts/mission_012_sandboxed_test_adapter.py"),
    Path("scripts/mission_012_egress_barrier_mcp_host.py"),
    Path("scripts/mission_012_mcp_supervisor.py"),
    Path("scripts/verify_mission_012_live_agent_postmerge.py"),
    Path("tests/test_mission_012_test_agent_adapter.py"),
    Path("tests/test_mission_012_sandboxed_test_adapter.py"),
    Path("tests/test_forgejo_live_pr_merge_e2e.py"),
    Path("tests/test_semantic_merge_canary.py"),
    Path("tests/test_mission_012_egress_barrier_mcp_host.py"),
    Path("src/bin/gate.rs"),
    Path("scripts/verify_cargo_license_state.py"),
    Path("scripts/test_cargo_license_state.py"),
    Path("src/webhook/http.rs"),
    Path("src/webhook.rs"),
    Path("src/webhook/reconcile.rs"),
    Path("tests/webhook_http_contract.rs"),
    Path("tests/webhook_handler_contract.rs"),
    Path("tests/webhook_reconcile_contract.rs"),
    Path("tests/webhook_receiver_process_e2e.rs"),
    Path("src/agent_runtime.rs"),
    Path("tests/agent_runtime_contract.rs"),
}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def latest(manifest: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if len(line) >= 67 and line[64:66] == "  ":
            checksum, name = line[:64], line[66:]
            if all(character in "0123456789abcdef" for character in checksum):
                values[name] = checksum
    return values


def append_current(manifest: Path, base: Path, paths: set[Path]) -> tuple[int, bytes]:
    old = manifest.read_bytes() if manifest.exists() else b""
    known = latest(manifest) if manifest.exists() else {}
    additions = []
    for relative in sorted(paths):
        path = base / relative
        if not path.is_file():
            raise FileNotFoundError(path)
        checksum = digest(path)
        name = relative.as_posix()
        if known.get(name) != checksum:
            additions.append(f"{checksum}  {name}\n")
    if additions:
        with manifest.open("ab") as stream:
            if old and not old.endswith(b"\n"):
                stream.write(b"\n")
            stream.writelines(line.encode("utf-8") for line in additions)
    if not manifest.read_bytes().startswith(old):
        raise RuntimeError(f"append-only prefix changed: {manifest}")
    return len(additions), old


def verify(manifest: Path, base: Path, paths: set[Path]) -> list[str]:
    known = latest(manifest)
    errors = []
    for relative in sorted(paths):
        path = base / relative
        if not path.is_file():
            errors.append(f"missing {relative}")
        elif known.get(relative.as_posix()) != digest(path):
            errors.append(f"stale {relative}")
    return errors


local_sets: dict[Path, set[Path]] = {}
for directory in RUNS:
    artifacts = {
        path.relative_to(directory)
        for path in directory.rglob("*")
        if path.is_file() and path.name not in LOCAL_EXCLUDED
    }
    local = directory / "SHA256SUMS"
    local.write_text(
        "".join(
            f"{digest(directory / relative)}  {relative.as_posix()}\n"
            for relative in sorted(artifacts)
        ),
        encoding="utf-8",
    )
    if verify(local, directory, artifacts):
        raise SystemExit(f"local manifest failed: {directory}")
    (directory / "manifest-check.txt").write_text(
        f"PASS: local SHA256SUMS verifies {len(artifacts)} evidence files recursively\n",
        encoding="utf-8",
    )
    local_sets[directory] = artifacts

required = {Path("Cargo.toml"), Path("docs/checkpoint.md"), Path("docs/architecture.md")} | SOURCE_PATHS
for directory, paths in local_sets.items():
    prefix = directory.relative_to(ROOT)
    required.update(prefix / relative for relative in paths)
    required.add(prefix / Path("SHA256SUMS"))
    required.add(prefix / Path("manifest-check.txt"))

product = ROOT / "evidence/SHA256SUMS"
parent = PARENT / "evidence/SHA256SUMS"
parent_required = {PREFIX / path for path in required} | {Path("docs/checkpoint.md")}
added_product, product_prefix = append_current(product, ROOT, required)
added_parent, parent_prefix = append_current(parent, PARENT, parent_required)
errors = verify(product, ROOT, required) + verify(parent, PARENT, parent_required)
if errors:
    raise SystemExit("\n".join(errors))

check = ROOT / "evidence/forgejo-agent-stdio-mcp-merge-001/global-manifest-check.txt"
check.write_text(
    f"PASS: append-only product manifest verifies {len(required)} current paths\n"
    f"PASS: append-only parent manifest verifies {len(parent_required)} current paths\n"
    "PASS: the two local manifests verify recursively\n"
    f"Appended changed hashes: product={added_product}, parent={added_parent}; prior prefixes preserved.\n",
    encoding="utf-8",
)
check_paths = {check.relative_to(ROOT)}
parent_check_paths = {PREFIX / check.relative_to(ROOT)}
append_current(product, ROOT, check_paths)
append_current(parent, PARENT, parent_check_paths)
if not product.read_bytes().startswith(product_prefix) or not parent.read_bytes().startswith(parent_prefix):
    raise SystemExit("append-only manifest prefix changed")
if verify(product, ROOT, required | check_paths) or verify(parent, PARENT, parent_required | parent_check_paths):
    raise SystemExit("current global manifest entry is stale")

for directory in RUNS:
    print((directory / "manifest-check.txt").read_text(encoding="utf-8"), end="")
print(check.read_text(encoding="utf-8"), end="")
