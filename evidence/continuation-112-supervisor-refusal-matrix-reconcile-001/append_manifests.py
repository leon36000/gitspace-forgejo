#!/usr/bin/env python3
"""Append current local evidence and documentation hashes without rewriting history."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))


def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha(path: Path) -> str:
    return sha_bytes(path.read_bytes())


def latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        digest, sep, name = line.partition("  ")
        if sep:
            result[name] = digest
    return result


def check_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    actual = sha_bytes(b"".join(lines[: expected["line_count"]]))
    if len(lines) < expected["line_count"] or actual != expected["sha256"]:
        raise SystemExit(f"{key} append-only prefix changed")


for index, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    if index.is_symlink() or not index.is_file():
        raise SystemExit(f"{key} manifest is missing or redirected")
    check_prefix(index, key)

local_latest = latest(LOCAL)
new_local_lines: list[str] = []
for path in sorted(ROOT.iterdir()):
    if path == LOCAL:
        continue
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"unexpected non-regular evidence entry: {path.name}")
    digest = sha(path)
    if local_latest.get(path.name) != digest:
        new_local_lines.append(f"{digest}  {path.name}\n")
if new_local_lines:
    with LOCAL.open("ab") as stream:
        stream.writelines(line.encode("ascii") for line in new_local_lines)
        stream.flush()

records: list[tuple[str, str, str]] = []
for rel, digest in latest(LOCAL).items():
    path = (ROOT / rel).resolve(strict=True)
    if ROOT.resolve() not in path.parents or sha(path) != digest:
        raise SystemExit(f"local evidence hash mismatch: {rel}")
    work_rel = path.relative_to(WORKTREE).as_posix()
    records.append((digest, work_rel, (Path(".runtime/webhook-005-workspace") / work_rel).as_posix()))

local_rel = LOCAL.relative_to(WORKTREE).as_posix()
records.append((sha(LOCAL), local_rel, (Path(".runtime/webhook-005-workspace") / local_rel).as_posix()))
for rel in ("docs/architecture.md", "docs/checkpoint.md", "docs/refusal-evidence-matrix.md"):
    digest = sha(WORKTREE / rel)
    records.append((digest, rel, (Path(".runtime/webhook-005-workspace") / rel).as_posix()))

work_latest = latest(WORKTREE_INDEX)
work_lines = [f"{digest}  {work_rel}\n" for digest, work_rel, _ in records if work_latest.get(work_rel) != digest]
if work_lines:
    with WORKTREE_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(work_lines)
        stream.flush()

project_records = [(digest, project_rel) for digest, _, project_rel in records]
project_records.append((sha(WORKTREE_INDEX), ".runtime/webhook-005-workspace/evidence/SHA256SUMS"))
project_latest = latest(PROJECT_INDEX)
project_lines = [f"{digest}  {rel}\n" for digest, rel in project_records if project_latest.get(rel) != digest]
if project_lines:
    with PROJECT_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(project_lines)
        stream.flush()

for index, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    check_prefix(index, key)
local_final = latest(LOCAL)
actual_local = {p.name for p in ROOT.iterdir() if p.is_file() and p != LOCAL}
if actual_local != set(local_final):
    raise SystemExit("local manifest does not cover every evidence file")
work_final = latest(WORKTREE_INDEX)
project_final = latest(PROJECT_INDEX)
for digest, work_rel, project_rel in records:
    if work_final.get(work_rel) != digest or project_final.get(project_rel) != digest:
        raise SystemExit(f"global manifest mismatch: {work_rel}")
if project_final.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != sha(WORKTREE_INDEX):
    raise SystemExit("parent manifest does not hash the current worktree evidence index")

print(json.dumps({
    "local_entries_appended": len(new_local_lines),
    "worktree_entries_appended": len(work_lines),
    "project_entries_appended": len(project_lines),
    "local_files_verified": len(actual_local),
    "worktree_prefix_preserved": True,
    "project_prefix_preserved": True,
}, sort_keys=True))
