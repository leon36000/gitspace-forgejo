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
    result: dict[str, str] = {}
    if manifest.exists():
        for line in manifest.read_text(encoding="utf-8").splitlines():
            if len(line) >= 67 and line[64:66] == "  ":
                checksum, name = line[:64], line[66:]
                if all(c in "0123456789abcdef" for c in checksum):
                    result[name] = checksum
    return result

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

local_manifest = RUN / "SHA256SUMS"
artifacts = {p.relative_to(RUN) for p in RUN.rglob("*") if p.is_file() and p.name not in EXCLUDED}
local_manifest.write_text("".join(f"{digest(RUN / p)}  {p.as_posix()}\n" for p in sorted(artifacts)), encoding="utf-8")
if verify(local_manifest, RUN, artifacts):
    raise SystemExit("local manifest failed")
(RUN / "manifest-check.txt").write_text(f"PASS: local manifest verifies {len(artifacts)} evidence files\n", encoding="utf-8")

required = {Path("docs/source-inventory.md"), Path("docs/architecture.md"), Path("docs/checkpoint.md"), Path("docs/refusal-evidence-matrix.md")}
required.update(RUN.relative_to(ROOT) / p for p in artifacts)
required.update({RUN.relative_to(ROOT) / Path("SHA256SUMS"), RUN.relative_to(ROOT) / Path("manifest-check.txt")})
product_manifest = ROOT / "evidence/SHA256SUMS"
parent_manifest = PARENT / "evidence/SHA256SUMS"
parent_required = {PREFIX / p for p in required} | {Path("docs/checkpoint.md")}
product_added, product_prefix = append_current(product_manifest, ROOT, required)
parent_added, parent_prefix = append_current(parent_manifest, PARENT, parent_required)
errors = verify(product_manifest, ROOT, required) + verify(parent_manifest, PARENT, parent_required)
if errors:
    raise SystemExit("\n".join(errors))
check = (
    f"PASS: product append-only manifest verifies {len(required)} paths\n"
    f"PASS: parent append-only manifest verifies {len(parent_required)} paths\n"
    f"PASS: local evidence manifest verifies {len(artifacts)} files\n"
    f"New hash records: product={product_added}, parent={parent_added}; prior prefixes preserved.\n"
)
(RUN / "global-manifest-check.txt").write_text(check, encoding="utf-8")
global_relative = RUN.relative_to(ROOT) / Path("global-manifest-check.txt")
product_added_check, product_prefix_after = append_current(product_manifest, ROOT, {global_relative})
parent_added_check, parent_prefix_after = append_current(parent_manifest, PARENT, {PREFIX / global_relative})
if not product_manifest.read_bytes().startswith(product_prefix) or not parent_manifest.read_bytes().startswith(parent_prefix):
    raise SystemExit("prior append-only manifest prefix changed")
errors = verify(product_manifest, ROOT, required | {global_relative}) + verify(parent_manifest, PARENT, parent_required | {PREFIX / global_relative})
if errors:
    raise SystemExit("\n".join(errors))
