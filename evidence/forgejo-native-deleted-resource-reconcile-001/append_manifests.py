#!/usr/bin/env python3
"""Append this receipt to local, worktree and project SHA-256 indexes."""
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


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        value, sep, name = line.partition("  ")
        if sep:
            result[name] = value
    return result


def check_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    if len(lines) < expected["line_count"] or digest(b"".join(lines[: expected["line_count"]])) != expected["sha256"]:
        raise SystemExit(f"{key} append-only prefix mismatch")


for path, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"{key} manifest is missing or redirected")
    check_prefix(path, key)

local_latest = latest(LOCAL)
local_lines: list[str] = []
for path in sorted(ROOT.iterdir()):
    if path == LOCAL:
        continue
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"unexpected non-regular evidence entry: {path.name}")
    value = digest(path.read_bytes())
    if local_latest.get(path.name) != value:
        local_lines.append(f"{value}  {path.name}\n")
if local_lines:
    with LOCAL.open("ab") as stream:
        stream.writelines(line.encode("ascii") for line in local_lines)
        stream.flush()

records: list[tuple[str, str, str]] = []
for name, value in latest(LOCAL).items():
    path = (ROOT / name).resolve(strict=True)
    if ROOT.resolve() not in path.parents or digest(path.read_bytes()) != value:
        raise SystemExit(f"local evidence hash mismatch: {name}")
    work_name = path.relative_to(WORKTREE).as_posix()
    project_name = (Path(".runtime/webhook-005-workspace") / work_name).as_posix()
    records.append((value, work_name, project_name))

local_name = LOCAL.relative_to(WORKTREE).as_posix()
records.append((digest(LOCAL.read_bytes()), local_name, (Path(".runtime/webhook-005-workspace") / local_name).as_posix()))
for name in ("docs/architecture.md", "docs/checkpoint.md", "docs/refusal-evidence-matrix.md"):
    value = digest((WORKTREE / name).read_bytes())
    records.append((value, name, (Path(".runtime/webhook-005-workspace") / name).as_posix()))

work_latest = latest(WORKTREE_INDEX)
work_lines = [f"{value}  {work_name}\n" for value, work_name, _ in records if work_latest.get(work_name) != value]
if work_lines:
    with WORKTREE_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(work_lines)
        stream.flush()

project_records = [(value, project_name) for value, _, project_name in records]
project_records.append((digest(WORKTREE_INDEX.read_bytes()), ".runtime/webhook-005-workspace/evidence/SHA256SUMS"))
project_latest = latest(PROJECT_INDEX)
project_lines = [f"{value}  {name}\n" for value, name in project_records if project_latest.get(name) != value]
if project_lines:
    with PROJECT_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(project_lines)
        stream.flush()

for path, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    check_prefix(path, key)
local_final = latest(LOCAL)
actual = {path.name for path in ROOT.iterdir() if path.is_file() and path != LOCAL}
if actual != set(local_final):
    raise SystemExit("local manifest does not cover every evidence file")
work_final = latest(WORKTREE_INDEX)
project_final = latest(PROJECT_INDEX)
for value, work_name, project_name in records:
    if work_final.get(work_name) != value or project_final.get(project_name) != value:
        raise SystemExit(f"global manifest mismatch: {work_name}")
if project_final.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not bind current worktree index")

print(json.dumps({
    "local_entries_appended": len(local_lines),
    "worktree_entries_appended": len(work_lines),
    "project_entries_appended": len(project_lines),
    "local_files_verified": len(actual),
    "worktree_prefix_preserved": True,
    "project_prefix_preserved": True,
}, sort_keys=True))
