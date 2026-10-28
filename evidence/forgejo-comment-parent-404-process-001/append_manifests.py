#!/usr/bin/env python3
"""Append local evidence and documentation hashes without rewriting old indexes."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORK = WORKTREE / "evidence/SHA256SUMS"
PARENT = PROJECT / "evidence/SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
DOCS = ("docs/checkpoint.md", "docs/refusal-evidence-matrix.md", "docs/source-inventory.md")


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        value, separator, name = line.partition("  ")
        if separator:
            result[name] = value
    return result


def check_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True) if path.exists() else []
    count = PREFIX[key]["line_count"]
    if len(lines) < count or sha(b"".join(lines[:count])) != PREFIX[key]["sha256"]:
        raise SystemExit(f"{key} append-only prefix mismatch")


for path, key in ((LOCAL, "local"), (WORK, "worktree"), (PARENT, "project")):
    if key != "local" and not path.is_file():
        raise SystemExit(f"missing {key} index")
    if path.exists() and (path.is_symlink() or not path.is_file()):
        raise SystemExit(f"invalid {key} index")
    check_prefix(path, key)

if not LOCAL.exists():
    LOCAL.touch(exist_ok=False)
local_latest = latest(LOCAL)
local_entries: list[str] = []
for path in sorted(ROOT.iterdir()):
    if path == LOCAL:
        continue
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"unexpected evidence entry: {path.name}")
    value = sha(path.read_bytes())
    if local_latest.get(path.name) != value:
        local_entries.append(f"{value}  {path.name}\n")
if local_entries:
    with LOCAL.open("a", encoding="ascii") as stream:
        stream.writelines(local_entries)

records: list[tuple[str, str, str]] = []
for name, value in latest(LOCAL).items():
    path = (ROOT / name).resolve(strict=True)
    if ROOT.resolve() not in path.parents or sha(path.read_bytes()) != value:
        raise SystemExit(f"local hash mismatch: {name}")
    work_name = path.relative_to(WORKTREE).as_posix()
    parent_name = (Path(".runtime/webhook-005-workspace") / work_name).as_posix()
    records.append((value, work_name, parent_name))

local_name = LOCAL.relative_to(WORKTREE).as_posix()
records.append((sha(LOCAL.read_bytes()), local_name, (Path(".runtime/webhook-005-workspace") / local_name).as_posix()))
for name in DOCS:
    value = sha((WORKTREE / name).read_bytes())
    records.append((value, name, (Path(".runtime/webhook-005-workspace") / name).as_posix()))

work_latest = latest(WORK)
work_entries = [f"{value}  {name}\n" for value, name, _ in records if work_latest.get(name) != value]
if work_entries:
    with WORK.open("a", encoding="ascii") as stream:
        stream.writelines(work_entries)
parent_records = [(value, parent_name) for value, _, parent_name in records]
parent_records.append((sha(WORK.read_bytes()), ".runtime/webhook-005-workspace/evidence/SHA256SUMS"))
parent_latest = latest(PARENT)
parent_entries = [f"{value}  {name}\n" for value, name in parent_records if parent_latest.get(name) != value]
if parent_entries:
    with PARENT.open("a", encoding="ascii") as stream:
        stream.writelines(parent_entries)

for path, key in ((LOCAL, "local"), (WORK, "worktree"), (PARENT, "project")):
    check_prefix(path, key)
local_final = latest(LOCAL)
actual = {path.name for path in ROOT.iterdir() if path.is_file() and path != LOCAL}
if actual != set(local_final):
    raise SystemExit("local index does not cover every evidence file")
work_final = latest(WORK)
parent_final = latest(PARENT)
for value, work_name, parent_name in records:
    if work_final.get(work_name) != value or parent_final.get(parent_name) != value:
        raise SystemExit(f"global index mismatch: {work_name}")
if parent_final.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != sha(WORK.read_bytes()):
    raise SystemExit("project index does not bind current worktree index")
print(json.dumps({"local_entries_appended": len(local_entries), "worktree_entries_appended": len(work_entries), "project_entries_appended": len(parent_entries), "local_files_verified": len(actual), "old_prefixes_preserved": True}, sort_keys=True))
