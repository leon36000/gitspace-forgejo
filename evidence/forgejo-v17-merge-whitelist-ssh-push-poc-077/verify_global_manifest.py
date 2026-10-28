from __future__ import annotations

import hashlib
from pathlib import Path

root = Path(__file__).resolve().parents[2]
evidence_dir = Path(__file__).resolve().parent
manifest = root / "evidence/SHA256SUMS"
required = {
    Path("docs/architecture.md"),
    Path("docs/source-inventory.md"),
    Path("docs/checkpoint.md"),
    Path("evidence/forgejo-v17-merge-whitelist-ssh-push-poc-077/SHA256SUMS"),
}
required.update(
    Path("evidence/forgejo-v17-merge-whitelist-ssh-push-poc-077") / path.name
    for path in evidence_dir.iterdir()
    if path.is_file()
    and path.name not in {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
)

latest: dict[str, str] = {}
for line in manifest.read_text().splitlines():
    if len(line) >= 67 and line[64:66] == "  ":
        digest, path = line[:64], line[66:]
        if all(character in "0123456789abcdef" for character in digest):
            latest[path] = digest

failures = []
for relative in sorted(required):
    path = root / relative
    if not path.is_file():
        failures.append(f"missing file: {relative}")
        continue
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if latest.get(relative.as_posix()) != actual:
        failures.append(f"latest global hash missing or stale: {relative}")

if failures:
    raise SystemExit("\n".join(failures))
print(f"PASS: latest append-only global hashes match {len(required)} current paths")
