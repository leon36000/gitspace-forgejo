#!/usr/bin/env python3
"""Append CONTINUATION-110 evidence and doc hashes without rewriting prefixes."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text())


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def latest(path: Path) -> dict[str, str]:
    result = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        digest, separator, rel = line.partition("  ")
        if separator:
            result[rel] = digest
    return result


def check_prefix(path: Path, key: str) -> None:
    data = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    if len(data) < expected["line_count"]:
        raise SystemExit(f"{key} manifest is shorter than its saved prefix")
    sha_prefix = hashlib.sha256(b"".join(data[: expected["line_count"]])).hexdigest()
    if sha_prefix != expected["sha256"]:
        raise SystemExit(f"{key} append-only prefix changed")


for index, key in ((WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    check_prefix(index, key)

records = []
for line in (ROOT / "SHA256SUMS").read_text(encoding="ascii").splitlines():
    digest, rel = line.split("  ", 1)
    path = (ROOT / rel).resolve(strict=True)
    if ROOT.resolve() not in path.parents or sha(path) != digest:
        raise SystemExit(f"local evidence hash mismatch: {rel}")
    rel_worktree = path.relative_to(WORKTREE).as_posix()
    rel_project = (Path(".runtime/webhook-005-workspace") / rel_worktree).as_posix()
    records.append((digest, rel_worktree, rel_project))

local_manifest_digest = sha(ROOT / "SHA256SUMS")
local_manifest_worktree = (ROOT / "SHA256SUMS").relative_to(WORKTREE).as_posix()
local_manifest_project = (
    Path(".runtime/webhook-005-workspace") / local_manifest_worktree
).as_posix()
records.append((local_manifest_digest, local_manifest_worktree, local_manifest_project))

for rel in ("docs/architecture.md", "docs/checkpoint.md"):
    path = WORKTREE / rel
    digest = sha(path)
    records.append((digest, rel, (Path(".runtime/webhook-005-workspace") / rel).as_posix()))

work_latest = latest(WORKTREE_INDEX)
work_lines = []
for digest, rel_worktree, _ in records:
    if work_latest.get(rel_worktree) != digest:
        work_lines.append(f"{digest}  {rel_worktree}\n")
if work_lines:
    with WORKTREE_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(work_lines)
        stream.flush()

project_latest = latest(PROJECT_INDEX)
project_lines = []
for digest, _, rel_project in records:
    if project_latest.get(rel_project) != digest:
        project_lines.append(f"{digest}  {rel_project}\n")
worktree_digest = sha(WORKTREE_INDEX)
worktree_rel = ".runtime/webhook-005-workspace/evidence/SHA256SUMS"
if project_latest.get(worktree_rel) != worktree_digest:
    project_lines.append(f"{worktree_digest}  {worktree_rel}\n")
if project_lines:
    with PROJECT_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(project_lines)
        stream.flush()

print(json.dumps({"worktree_entries_appended": len(work_lines),
                  "project_entries_appended": len(project_lines),
                  "worktree_line_count": len(WORKTREE_INDEX.read_text().splitlines()),
                  "project_line_count": len(PROJECT_INDEX.read_text().splitlines())}, sort_keys=True))
