#!/usr/bin/env python3
"""Append CONTINUATION-384 evidence and documentation hashes."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
DOCS = (
    "docs/checkpoint.md",
    "docs/implementation-plan.md",
    "docs/refusal-evidence-matrix.md",
    "docs/architecture.md",
    "docs/source-inventory.md",
)
EXCLUDED_NAMES = {"manifest-check.txt", "global-manifest-check.txt"}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def append(path: Path, values: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in values:
            stream.write(f"{value}  {name}\n".encode("utf-8"))


if LOCAL.exists() or LOCAL.is_symlink():
    raise SystemExit("local evidence index already exists")
prefix = {}
for key, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"append-only index unavailable: {path}")
    data = path.read_bytes()
    prefix[key] = {"line_count": len(data.splitlines()), "sha256": digest(data)}
(ROOT / "manifest-prefix-before.json").write_text(
    json.dumps(prefix, sort_keys=True, indent=2) + "\n", encoding="utf-8"
)

files = []
for path in ROOT.rglob("*"):
    if "__pycache__" in path.parts:
        continue
    if path.is_symlink():
        raise SystemExit(f"symlink in evidence directory: {path.relative_to(ROOT)}")
    if path.is_file() and path != LOCAL and path.name not in EXCLUDED_NAMES:
        files.append(path)
files.sort(key=lambda path: path.relative_to(ROOT).as_posix())
entries = [(digest(path.read_bytes()), path.relative_to(ROOT).as_posix()) for path in files]
LOCAL.write_text("".join(f"{value}  {name}\n" for value, name in entries), encoding="utf-8")
local_hash = digest(LOCAL.read_bytes())
work_entries = [(value, f"evidence/{ROOT.name}/{name}") for value, name in entries]
work_entries.append((local_hash, f"evidence/{ROOT.name}/SHA256SUMS"))
project_entries = [
    (value, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/{name}")
    for value, name in entries
]
project_entries.append(
    (local_hash, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS")
)
for relative in DOCS:
    value = digest((WORKTREE / relative).read_bytes())
    work_entries.append((value, relative))
    project_entries.append((value, f".runtime/webhook-005-workspace/{relative}"))
append(WORKTREE_INDEX, work_entries)
append(PROJECT_INDEX, project_entries)
append(
    PROJECT_INDEX,
    [(digest(WORKTREE_INDEX.read_bytes()), ".runtime/webhook-005-workspace/evidence/SHA256SUMS")],
)
print(json.dumps({
    "local_evidence_files": len(entries),
    "worktree_entries_appended": len(work_entries),
    "project_entries_appended": len(project_entries) + 1,
    "prefixes_recorded": prefix,
}, sort_keys=True))
