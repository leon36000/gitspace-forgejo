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
            if all(char in "0123456789abcdef" for char in value):
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
    current = manifest.read_bytes()
    if not current.startswith(old):
        raise RuntimeError(f"manifest prefix changed: {manifest}")
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
    path for path in EVIDENCE.iterdir()
    if path.is_file() and path.name not in {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
)
LOCAL.write_text("".join(f"{digest(path)}  {path.name}\n" for path in artifacts))
for line in LOCAL.read_text().splitlines():
    value, name = line.split("  ", 1)
    if digest(EVIDENCE / name) != value:
        raise SystemExit(f"local evidence checksum mismatch: {name}")
LOCAL_CHECK.write_text(f"PASS: local SHA256SUMS verifies {len(artifacts)} evidence files\n")

core = {
    Path("Cargo.toml"), Path("Cargo.lock"),
    Path("docs/checkpoint.md"),
    Path("tests/webhook_receiver_process_e2e.rs"),
}
run = {
    EVIDENCE.relative_to(ROOT) / path.name
    for path in EVIDENCE.iterdir()
    if path.is_file() and path.name != "global-manifest-check.txt"
}
required = core | run
product = ROOT / "evidence/SHA256SUMS"
parent = PARENT / "evidence/SHA256SUMS"
added_product, product_prefix = append_current(product, ROOT, required)
parent_required = {PREFIX / path for path in required} | {Path("docs/checkpoint.md")}
added_parent, parent_prefix = append_current(parent, PARENT, parent_required)
errors = verify(product, ROOT, required) + verify(parent, PARENT, parent_required)
if errors:
    raise SystemExit("\n".join(errors))
GLOBAL_CHECK.write_text(
    f"PASS: current append-only hashes match {len(required)} product paths\n"
    f"PASS: current parent append-only hashes match {len(parent_required)} paths\n"
    f"Appended changed hashes: product={added_product}, parent={added_parent}; prior prefixes preserved.\n"
)
append_current(product, ROOT, {GLOBAL_CHECK.relative_to(ROOT)})
append_current(parent, PARENT, {PREFIX / GLOBAL_CHECK.relative_to(ROOT)})
if not product.read_bytes().startswith(product_prefix) or not parent.read_bytes().startswith(parent_prefix):
    raise SystemExit("append-only manifest prefix changed")
print(GLOBAL_CHECK.read_text(), end="")
