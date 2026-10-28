from __future__ import annotations

import hashlib
from pathlib import Path

EVIDENCE = Path(__file__).resolve().parent
ROOT = EVIDENCE.parents[1]
PARENT = ROOT.parents[1]
PREFIX = ROOT.relative_to(PARENT)
RUNS = [
    ROOT / "evidence" / f"forgejo-agent-egress-stdio-mcp-{number}"
    for number in ("001", "002", "003", "004", "005", "006", "007")
]
EXCLUDED = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def latest(manifest: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    if not manifest.exists():
        return values
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if len(line) >= 67 and line[64:66] == "  ":
            checksum, name = line[:64], line[66:]
            if all(character in "0123456789abcdef" for character in checksum):
                values[name] = checksum
    return values


def append_current(manifest: Path, base: Path, paths: set[Path]) -> tuple[int, bytes]:
    old = manifest.read_bytes() if manifest.exists() else b""
    known = latest(manifest)
    additions = []
    for relative in sorted(paths):
        path = base / relative
        if not path.is_file():
            raise FileNotFoundError(path)
        name = relative.as_posix()
        checksum = digest(path)
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


local_artifacts: dict[Path, set[Path]] = {}
for directory in RUNS:
    if not directory.is_dir():
        raise FileNotFoundError(directory)
    artifacts = {
        path.relative_to(directory)
        for path in directory.rglob("*")
        if path.is_file() and path.name not in EXCLUDED
    }
    local = directory / "SHA256SUMS"
    local.write_text(
        "".join(f"{digest(directory / relative)}  {relative.as_posix()}\n" for relative in sorted(artifacts)),
        encoding="utf-8",
    )
    errors = verify(local, directory, artifacts)
    if errors:
        raise SystemExit("\n".join(errors))
    (directory / "manifest-check.txt").write_text(
        f"PASS: local SHA256SUMS verifies {len(artifacts)} evidence files recursively\n",
        encoding="utf-8",
    )
    local_artifacts[directory] = artifacts

core = {
    Path("Cargo.toml"),
    Path("docs/checkpoint.md"),
    Path("src/bin/forgejo-reconcile-local.rs"),
}
required = set(core)
for directory, artifacts in local_artifacts.items():
    required.update(directory.relative_to(ROOT) / relative for relative in artifacts)
    required.add(directory.relative_to(ROOT) / Path("manifest-check.txt"))

product = ROOT / "evidence/SHA256SUMS"
parent = PARENT / "evidence/SHA256SUMS"
parent_required = {PREFIX / path for path in required} | {Path("docs/checkpoint.md")}
added_product, product_prefix = append_current(product, ROOT, required)
added_parent, parent_prefix = append_current(parent, PARENT, parent_required)
errors = verify(product, ROOT, required) + verify(parent, PARENT, parent_required)
if errors:
    raise SystemExit("\n".join(errors))

global_check = EVIDENCE / "global-manifest-check.txt"
global_check.write_text(
    f"PASS: current append-only product hashes match {len(required)} paths\n"
    f"PASS: current append-only parent hashes match {len(parent_required)} paths\n"
    f"PASS: seven local run manifests verify recursively\n"
    f"Appended changed hashes: product={added_product}, parent={added_parent}; prior prefixes preserved.\n",
    encoding="utf-8",
)
append_current(product, ROOT, {global_check.relative_to(ROOT)})
append_current(parent, PARENT, {PREFIX / global_check.relative_to(ROOT)})
if not product.read_bytes().startswith(product_prefix) or not parent.read_bytes().startswith(parent_prefix):
    raise SystemExit("append-only manifest prefix changed")
if verify(product, ROOT, required | {global_check.relative_to(ROOT)}) or verify(
    parent, PARENT, parent_required | {PREFIX / global_check.relative_to(ROOT)}
):
    raise SystemExit("current global manifest entry is stale")

for directory in RUNS:
    print((directory / "manifest-check.txt").read_text(encoding="utf-8"), end="")
print(global_check.read_text(encoding="utf-8"), end="")
