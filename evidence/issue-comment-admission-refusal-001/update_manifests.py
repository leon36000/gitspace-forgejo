from __future__ import annotations

import hashlib
from pathlib import Path


RUN = Path(__file__).resolve().parent
ROOT = RUN.parents[1]
PARENT = ROOT.parents[1]
PREFIX = ROOT.relative_to(PARENT)
EXCLUDED = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def latest(manifest: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    if not manifest.exists():
        return values
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if len(line) < 67 or line[64:66] != "  ":
            continue
        checksum, name = line[:64], line[66:]
        if all(character in "0123456789abcdef" for character in checksum):
            values[name] = checksum
    return values


def append_current(manifest: Path, base: Path, paths: set[Path]) -> tuple[int, bytes]:
    old = manifest.read_bytes() if manifest.exists() else b""
    known = latest(manifest)
    additions: list[str] = []
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
            stream.writelines(item.encode("utf-8") for item in additions)
    if not manifest.read_bytes().startswith(old):
        raise RuntimeError(f"append-only prefix changed: {manifest}")
    return len(additions), old


def verify(manifest: Path, base: Path, paths: set[Path]) -> list[str]:
    known = latest(manifest)
    errors: list[str] = []
    for relative in sorted(paths):
        path = base / relative
        if not path.is_file():
            errors.append(f"missing {relative}")
        elif known.get(relative.as_posix()) != digest(path):
            errors.append(f"stale {relative}")
    return errors


artifacts = {
    path.relative_to(RUN)
    for path in RUN.rglob("*")
    if path.is_file() and path.name not in EXCLUDED
}
local_manifest = RUN / "SHA256SUMS"
local_manifest.write_text(
    "".join(
        f"{digest(RUN / relative)}  {relative.as_posix()}\n"
        for relative in sorted(artifacts)
    ),
    encoding="utf-8",
)
if verify(local_manifest, RUN, artifacts):
    raise SystemExit("local run manifest failed verification")
(RUN / "manifest-check.txt").write_text(
    f"PASS: local SHA256SUMS verifies {len(artifacts)} evidence files recursively\n",
    encoding="utf-8",
)

required = {
    Path("docs/architecture.md"),
    Path("docs/checkpoint.md"),
    Path("docs/refusal-evidence-matrix.md"),
    Path("src/mission_admission.rs"),
    Path("tests/webhook_mission_admission_contract.rs"),
}
required.update(RUN.relative_to(ROOT) / relative for relative in artifacts)
required.add(RUN.relative_to(ROOT) / Path("manifest-check.txt"))
product_manifest = ROOT / "evidence/SHA256SUMS"
parent_manifest = PARENT / "evidence/SHA256SUMS"
parent_required = {PREFIX / path for path in required} | {Path("docs/checkpoint.md")}
added_product, product_prefix = append_current(product_manifest, ROOT, required)
added_parent, parent_prefix = append_current(parent_manifest, PARENT, parent_required)
errors = verify(product_manifest, ROOT, required) + verify(parent_manifest, PARENT, parent_required)
if errors:
    raise SystemExit("\n".join(errors))

global_check = RUN / "global-manifest-check.txt"
global_check.write_text(
    f"PASS: current append-only product hashes match {len(required)} paths\n"
    f"PASS: current append-only parent hashes match {len(parent_required)} paths\n"
    f"PASS: local run manifest verifies {len(artifacts)} files recursively\n"
    f"Appended changed hashes: product={added_product}, parent={added_parent}; prior prefixes preserved.\n",
    encoding="utf-8",
)
global_relative = global_check.relative_to(ROOT)
append_current(product_manifest, ROOT, {global_relative})
append_current(parent_manifest, PARENT, {PREFIX / global_relative})
if not product_manifest.read_bytes().startswith(product_prefix):
    raise SystemExit("product manifest prefix changed")
if not parent_manifest.read_bytes().startswith(parent_prefix):
    raise SystemExit("parent manifest prefix changed")
if verify(product_manifest, ROOT, required | {global_relative}):
    raise SystemExit("current product manifest entry is stale")
if verify(parent_manifest, PARENT, parent_required | {PREFIX / global_relative}):
    raise SystemExit("current parent manifest entry is stale")

print((RUN / "manifest-check.txt").read_text(encoding="utf-8"), end="")
print(global_check.read_text(encoding="utf-8"), end="")
