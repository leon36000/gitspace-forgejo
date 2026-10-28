#!/usr/bin/env python3
"""Append CONT-213 evidence and current-document hashes without rewriting prefixes."""

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
LOCAL_INDEX = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
EXCLUDED_NAMES = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
DOCUMENTS = (
    "docs/architecture.md",
    "docs/checkpoint.md",
    "docs/implementation-plan.md",
    "docs/refusal-evidence-matrix.md",
    "docs/source-inventory.md",
)


def digest_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def digest_file(path: Path) -> str:
    return digest_bytes(path.read_bytes())


def parse_latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        digest, separator, name = line.partition("  ")
        if separator:
            result[name] = digest
    return result


def verify_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True) if path.exists() else []
    expected = PREFIX[key]
    if (
        len(lines) < expected["line_count"]
        or digest_bytes(b"".join(lines[: expected["line_count"]])) != expected["sha256"]
    ):
        raise SystemExit(f"{key} append-only prefix changed")


def append_if_needed(path: Path, relative: str, digest: str) -> None:
    latest = parse_latest(path)
    if latest.get(relative) == digest:
        return
    with path.open("ab") as output:
        output.write(f"{digest}  {relative}\n".encode("utf-8"))


verify_prefix(WORKTREE_INDEX, "worktree")
verify_prefix(PROJECT_INDEX, "project")
verify_prefix(LOCAL_INDEX, "local")

actual: dict[str, str] = {}
for path in ROOT.rglob("*"):
    if path.is_symlink():
        raise SystemExit(f"symlink in local evidence: {path.relative_to(ROOT)}")
    if path.is_file() and path != LOCAL_INDEX and path.name not in EXCLUDED_NAMES:
        actual[path.relative_to(ROOT).as_posix()] = digest_file(path)

for relative, digest in sorted(actual.items()):
    append_if_needed(LOCAL_INDEX, relative, digest)

for relative, digest in sorted(actual.items()):
    worktree_relative = (ROOT / relative).relative_to(WORKTREE).as_posix()
    project_relative = (Path(".runtime/webhook-005-workspace") / worktree_relative).as_posix()
    append_if_needed(WORKTREE_INDEX, worktree_relative, digest)
    append_if_needed(PROJECT_INDEX, project_relative, digest)

local_relative = LOCAL_INDEX.relative_to(WORKTREE).as_posix()
append_if_needed(WORKTREE_INDEX, local_relative, digest_file(LOCAL_INDEX))
append_if_needed(
    PROJECT_INDEX,
    (Path(".runtime/webhook-005-workspace") / local_relative).as_posix(),
    digest_file(LOCAL_INDEX),
)

for document in DOCUMENTS:
    digest = digest_file(WORKTREE / document)
    append_if_needed(WORKTREE_INDEX, document, digest)
    append_if_needed(
        PROJECT_INDEX,
        (Path(".runtime/webhook-005-workspace") / document).as_posix(),
        digest,
    )

append_if_needed(PROJECT_INDEX, ".runtime/webhook-005-workspace/evidence/SHA256SUMS", digest_file(WORKTREE_INDEX))
print(json.dumps({
    "local_files": len(actual),
    "local_manifest_sha256": digest_file(LOCAL_INDEX),
    "worktree_index_sha256": digest_file(WORKTREE_INDEX),
    "project_index_sha256": digest_file(PROJECT_INDEX),
    "prefixes_preserved": True,
}, sort_keys=True))
