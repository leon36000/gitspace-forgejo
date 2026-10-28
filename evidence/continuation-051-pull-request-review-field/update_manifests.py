from __future__ import annotations

import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(__file__).resolve().parent
PARENT = ROOT.parents[1]
PREFIX = ROOT.relative_to(PARENT)
LOCAL = EVIDENCE / "SHA256SUMS"
LOCAL_CHECK = EVIDENCE / "manifest-check.txt"
GLOBAL_CHECK = EVIDENCE / "global-manifest-check.txt"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def latest(manifest: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in manifest.read_text().splitlines():
        if len(line) >= 67 and line[64:66] == "  ":
            value, name = line[:64], line[66:]
            if all(character in "0123456789abcdef" for character in value):
                result[name] = value
    return result


def append_current(manifest: Path, base: Path, paths: set[Path]) -> tuple[int, bytes]:
    old = manifest.read_bytes()
    known = latest(manifest)
    additions = []
    for relative in sorted(paths):
        path = base / relative
        if not path.is_file():
            raise FileNotFoundError(path)
        name = relative.as_posix()
        value = digest(path)
        if known.get(name) != value:
            additions.append(f"{value}  {name}\n")
    if additions:
        with manifest.open("ab") as stream:
            if old and not old.endswith(b"\n"):
                stream.write(b"\n")
            stream.writelines(line.encode() for line in additions)
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


artifacts = sorted(
    path
    for path in EVIDENCE.iterdir()
    if path.is_file() and path.name not in {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
)
LOCAL.write_text("".join(f"{digest(path)}  {path.name}\n" for path in artifacts))
local_errors = verify(LOCAL, EVIDENCE, {Path(path.name) for path in artifacts})
if local_errors:
    raise SystemExit("\n".join(local_errors))
LOCAL_CHECK.write_text(f"PASS: local SHA256SUMS verifies {len(artifacts)} evidence files\n")

core = {
    Path("Cargo.toml"),
    Path("docs/checkpoint.md"),
    Path("src/webhook.rs"),
    Path("src/webhook/reconcile.rs"),
    Path("tests/webhook_reconcile_contract.rs"),
    Path("tests/webhook_handler_contract.rs"),
}
run = {
    EVIDENCE.relative_to(ROOT) / path.name
    for path in EVIDENCE.iterdir()
    if path.is_file() and path.name not in {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
}
required = core | run
product = ROOT / "evidence/SHA256SUMS"
parent = PARENT / "evidence/SHA256SUMS"
parent_required = {PREFIX / path for path in required} | {Path("docs/checkpoint.md")}

added_product, product_prefix = append_current(product, ROOT, required)
added_parent, parent_prefix = append_current(parent, PARENT, parent_required)
errors = verify(product, ROOT, required) + verify(parent, PARENT, parent_required)
if errors:
    raise SystemExit("\n".join(errors))

GLOBAL_CHECK.write_text(
    f"PASS: current append-only product hashes match {len(required)} paths\n"
    f"PASS: current append-only parent hashes match {len(parent_required)} paths\n"
    f"Appended changed hashes: product={added_product}, parent={added_parent}; prior prefixes preserved.\n"
)

append_current(product, ROOT, {GLOBAL_CHECK.relative_to(ROOT)})
append_current(parent, PARENT, {PREFIX / GLOBAL_CHECK.relative_to(ROOT)})
if not product.read_bytes().startswith(product_prefix) or not parent.read_bytes().startswith(parent_prefix):
    raise SystemExit("append-only manifest prefix changed")
if verify(product, ROOT, required | {GLOBAL_CHECK.relative_to(ROOT)}) or verify(
    parent, PARENT, parent_required | {PREFIX / GLOBAL_CHECK.relative_to(ROOT)}
):
    raise SystemExit("current global manifest entry is stale")

print(LOCAL_CHECK.read_text(), end="")
print(GLOBAL_CHECK.read_text(), end="")
