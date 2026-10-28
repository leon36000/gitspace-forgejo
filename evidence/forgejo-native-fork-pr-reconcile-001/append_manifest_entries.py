#!/usr/bin/env python3
"""Append this run's hashes while preserving the saved worktree/project prefixes."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text())
LOCAL_INDEX = ROOT / "SHA256SUMS"

INDEXED_PATHS = (
    "docs/architecture.md",
    "docs/checkpoint.md",
    "docs/source-inventory.md",
    "docs/refusal-evidence-matrix.md",
    "scripts/run_forgejo_live_webhook_reconcile_emitted.py",
    "tests/test_forgejo_emitted_webhook_e2e.py",
)


def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha(path: Path) -> str:
    return sha_bytes(path.read_bytes())


def latest(path: Path) -> dict[str, str]:
    result = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        digest, separator, rel = line.partition("  ")
        if separator:
            result[rel] = digest
    return result


def check_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    if len(lines) < expected["line_count"]:
        raise SystemExit(f"{key} index is shorter than the saved prefix")
    actual = sha_bytes(b"".join(lines[: expected["line_count"]]))
    if actual != expected["sha256"]:
        raise SystemExit(f"{key} append-only prefix changed")


for index, key in ((WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    if index.is_symlink() or not index.is_file():
        raise SystemExit(f"{key} index is missing or redirected")
    check_prefix(index, key)

local_latest = latest(LOCAL_INDEX)
check_prefix(LOCAL_INDEX, "local")
local_lines = []
for path in sorted(ROOT.iterdir()):
    if path.name == "SHA256SUMS":
        continue
    # py_compile left this non-evidence cache; validate its narrow shape and retain it.
    if path.name == "__pycache__":
        if path.is_symlink() or not path.is_dir() or any(
            child.is_symlink() or not child.is_file() or child.suffix != ".pyc"
            for child in path.iterdir()
        ):
            raise SystemExit("unexpected contents in generated Python cache")
        continue
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"unexpected non-regular evidence entry: {path.name}")
    digest = sha(path)
    if local_latest.get(path.name) != digest:
        local_lines.append(f"{digest}  {path.name}\n")
if local_lines:
    with LOCAL_INDEX.open("ab") as stream:
        stream.writelines(line.encode("ascii") for line in local_lines)
        stream.flush()

records = []
for rel, digest in latest(LOCAL_INDEX).items():
    path = (ROOT / rel).resolve(strict=True)
    if ROOT.resolve() not in path.parents or sha(path) != digest:
        raise SystemExit(f"local evidence checksum mismatch: {rel}")
    work_rel = path.relative_to(WORKTREE).as_posix()
    project_rel = (Path(".runtime/webhook-005-workspace") / work_rel).as_posix()
    records.append((digest, work_rel, project_rel))

local_manifest_digest = sha(LOCAL_INDEX)
local_manifest_work = LOCAL_INDEX.relative_to(WORKTREE).as_posix()
records.append((local_manifest_digest, local_manifest_work,
                (Path(".runtime/webhook-005-workspace") / local_manifest_work).as_posix()))
for rel in INDEXED_PATHS:
    digest = sha(WORKTREE / rel)
    records.append((digest, rel, (Path(".runtime/webhook-005-workspace") / rel).as_posix()))

work_latest = latest(WORKTREE_INDEX)
work_lines = [f"{digest}  {rel}\n" for digest, rel, _ in records if work_latest.get(rel) != digest]
if work_lines:
    with WORKTREE_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(work_lines)
        stream.flush()

project_latest = latest(PROJECT_INDEX)
project_lines = [f"{digest}  {rel}\n" for digest, _, rel in records
                 if project_latest.get(rel) != digest]
worktree_rel = ".runtime/webhook-005-workspace/evidence/SHA256SUMS"
worktree_digest = sha(WORKTREE_INDEX)
if project_latest.get(worktree_rel) != worktree_digest:
    project_lines.append(f"{worktree_digest}  {worktree_rel}\n")
if project_lines:
    with PROJECT_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(project_lines)
        stream.flush()

print(json.dumps({
    "local_entries_appended": len(local_lines),
    "worktree_entries_appended": len(work_lines),
    "project_entries_appended": len(project_lines),
    "worktree_lines": len(WORKTREE_INDEX.read_text().splitlines()),
    "project_lines": len(PROJECT_INDEX.read_text().splitlines()),
}, sort_keys=True))
