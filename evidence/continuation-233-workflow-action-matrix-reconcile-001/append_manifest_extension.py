#!/usr/bin/env python3
"""Append verifier artifacts to CONT-233 manifests without rewriting history."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path


ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for digest, name in entries:
            stream.write(f"{digest}  {name}\n".encode())
        stream.flush()
        os.fsync(stream.fileno())


def latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_bytes().splitlines():
        try:
            digest, name = line.decode().split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        result[name] = digest
    return result


receipt = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
for key, path in (("local", LOCAL), ("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"index unavailable: {path}")
    data = path.read_bytes()
    prefix_lines = data.splitlines(keepends=True)[: receipt[key]["line_count"]]
    if sha(b"".join(prefix_lines)) != receipt[key]["sha256"]:
        raise SystemExit(f"{key} historical prefix changed")

for line in LOCAL.read_text(encoding="utf-8").splitlines():
    expected, name = line.split("  ", 1)
    path = ROOT / name
    if not path.is_file() or path.is_symlink() or sha(path.read_bytes()) != expected:
        raise SystemExit(f"existing local manifest mismatch: {name}")

new_names = ("verify_reconcile.py", "append_manifest_extension.py")
local_additions = []
for name in new_names:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"extension artifact unavailable: {name}")
    local_additions.append((sha(path.read_bytes()), name))
append(LOCAL, local_additions)
local_digest = sha(LOCAL.read_bytes())

relative = f"evidence/{ROOT.name}/"
worktree_additions = [(digest, relative + name) for digest, name in local_additions]
worktree_additions.append((local_digest, relative + "SHA256SUMS"))
append(WORKTREE_INDEX, worktree_additions)

project_relative = WORKTREE.relative_to(PROJECT).as_posix() + "/" + relative
project_additions = [(digest, project_relative + name) for digest, name in local_additions]
project_additions.append((local_digest, project_relative + "SHA256SUMS"))
project_additions.append((sha(WORKTREE_INDEX.read_bytes()), f"{WORKTREE.relative_to(PROJECT).as_posix()}/evidence/SHA256SUMS"))
append(PROJECT_INDEX, project_additions)

for key, path in (("local", LOCAL), ("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = path.read_bytes()
    lines = data.splitlines(keepends=True)
    prefix = b"".join(lines[: receipt[key]["line_count"]])
    if sha(prefix) != receipt[key]["sha256"]:
        raise SystemExit(f"{key} prefix changed during append")

worktree_latest = latest(WORKTREE_INDEX)
project_latest = latest(PROJECT_INDEX)
for digest, name in local_additions:
    if worktree_latest.get(relative + name) != digest or project_latest.get(project_relative + name) != digest:
        raise SystemExit(f"latest global entry mismatch: {name}")
if worktree_latest.get(relative + "SHA256SUMS") != local_digest:
    raise SystemExit("latest worktree local-manifest checksum mismatch")
if project_latest.get(project_relative + "SHA256SUMS") != local_digest:
    raise SystemExit("latest project local-manifest checksum mismatch")

print(
    json.dumps(
        {
            "local_extension_entries": len(local_additions),
            "worktree_project_prefixes_preserved": True,
            "latest_extension_entries_match": True,
        },
        sort_keys=True,
    )
)
