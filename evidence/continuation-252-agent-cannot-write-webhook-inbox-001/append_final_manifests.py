#!/usr/bin/env python3
"""Tolerantly verify and finish the already-appended CONT-252 indexes."""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
PREFIX = ROOT / "manifest-prefix-before.json"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
REPORT = ROOT / "manifest-verification.md"
FINALIZER = ROOT / "append_final_manifests.py"

def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()

def parsed(path: Path) -> tuple[dict[str, str], int, int]:
    latest: dict[str, str] = {}
    blank = malformed = 0
    for raw in path.read_bytes().splitlines():
        if not raw.strip():
            blank += 1
            continue
        try:
            decoded = raw.decode("utf-8")
            value, name = decoded.split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            malformed += 1
            continue
        if len(value) != 64 or any(c not in "0123456789abcdef" for c in value) or not name:
            malformed += 1
            continue
        latest[name] = value
    return latest, blank, malformed

def append(path: Path, values: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in values:
            stream.write(f"{value}  {name}\n".encode())
        stream.flush()
        os.fsync(stream.fileno())

def append_report_to_indexes() -> tuple[int, int, int]:
    report_hash = digest(REPORT.read_bytes())
    with LOCAL.open("ab") as stream:
        stream.write(f"{report_hash}  manifest-verification.md\n".encode())
        stream.flush()
        os.fsync(stream.fileno())
    local_hash = digest(LOCAL.read_bytes())
    evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
    project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
    worktree_values = [
        (report_hash, f"{evidence_prefix}/manifest-verification.md"),
        (local_hash, f"{evidence_prefix}/SHA256SUMS"),
    ]
    append(WORKTREE_INDEX, worktree_values)
    project_values = [
        (report_hash, f"{project_prefix}/{evidence_prefix}/manifest-verification.md"),
        (local_hash, f"{project_prefix}/{evidence_prefix}/SHA256SUMS"),
        (digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS"),
    ]
    append(PROJECT_INDEX, project_values)
    return len(worktree_values), len(project_values), local_hash

if not LOCAL.is_file() or not PREFIX.is_file() or REPORT.exists():
    raise SystemExit("first append evidence is unavailable or final report already exists")
before = json.loads(PREFIX.read_text(encoding="utf-8"))
for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = path.read_bytes()
    prior = before[label]
    lines = data.splitlines(keepends=True)
    if digest(b"".join(lines[:prior["count"]])) != prior["sha256"]:
        raise SystemExit(f"original {label} prefix changed")

local_latest, _, local_malformed = parsed(LOCAL)
if local_malformed:
    raise SystemExit("local evidence index contains a malformed line")
for name, value in local_latest.items():
    if name == "local-run.md":
        continue  # the report was updated after the first append; append its new hash below
    path = ROOT / name
    if not path.is_file() or digest(path.read_bytes()) != value:
        raise SystemExit(f"local evidence hash mismatch: {name}")
for needed in ("append_manifests.py", "append_final_manifests.py", "manifest-prefix-before.json", "local-run.md"):
    if needed != "append_final_manifests.py" and needed not in local_latest:
        if needed == "manifest-prefix-before.json":
            continue  # first index did not include the prefix receipt
        raise SystemExit(f"initial local manifest omitted {needed}")

source_values: list[tuple[str, str]] = []
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = line.split("  ", 1)
    path = WORKTREE / name
    if not path.is_file() or digest(path.read_bytes()) != value:
        raise SystemExit(f"source hash is stale: {name}")
    source_values.append((value, name))

worktree_latest, worktree_blank, worktree_malformed = parsed(WORKTREE_INDEX)
project_latest, project_blank, project_malformed = parsed(PROJECT_INDEX)
evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
for name, value in local_latest.items():
    if name == "local-run.md":
        continue
    if worktree_latest.get(f"{evidence_prefix}/{name}") != value:
        raise SystemExit(f"worktree index lacks first local append: {name}")
    if project_latest.get(f"{project_prefix}/{evidence_prefix}/{name}") != value:
        raise SystemExit(f"project index lacks first local append: {name}")
for value, name in source_values:
    if worktree_latest.get(name) != value or project_latest.get(f"{project_prefix}/{name}") != value:
        raise SystemExit(f"source manifest entry mismatch: {name}")

# Finish the first transaction append-only, preserving every earlier row.
final_local_values: list[tuple[str, str]] = []
for name in ("append_final_manifests.py", "manifest-prefix-before.json", "local-run.md"):
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"final evidence unavailable: {name}")
    final_local_values.append((digest(path.read_bytes()), name))
append(LOCAL, final_local_values)
local_hash = digest(LOCAL.read_bytes())
worktree_values = [(value, f"{evidence_prefix}/{name}") for value, name in final_local_values]
worktree_values.append((local_hash, f"{evidence_prefix}/SHA256SUMS"))
worktree_values.extend(source_values)
append(WORKTREE_INDEX, worktree_values)
project_values = [(value, f"{project_prefix}/{evidence_prefix}/{name}") for value, name in final_local_values]
project_values.append((local_hash, f"{project_prefix}/{evidence_prefix}/SHA256SUMS"))
project_values.extend((value, f"{project_prefix}/{name}") for value, name in source_values)
append(PROJECT_INDEX, project_values)
project_index_value = (digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS")
append(PROJECT_INDEX, [project_index_value])

# Confirm all original prefixes and the just-appended entries before writing the final verification note.
for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = path.read_bytes(); lines = data.splitlines(keepends=True); prior = before[label]
    if digest(b"".join(lines[:prior["count"]])) != prior["sha256"]:
        raise SystemExit(f"original {label} prefix changed during final append")
local_latest, _, _ = parsed(LOCAL)
worktree_latest, _, _ = parsed(WORKTREE_INDEX)
project_latest, _, _ = parsed(PROJECT_INDEX)
for value, name in final_local_values:
    if local_latest.get(name) != value or worktree_latest.get(f"{evidence_prefix}/{name}") != value or project_latest.get(f"{project_prefix}/{evidence_prefix}/{name}") != value:
        raise SystemExit(f"final local entry failed verification: {name}")
if local_latest.get("local-run.md") != digest((ROOT / "local-run.md").read_bytes()):
    raise SystemExit("final local-run hash failed verification")
for value, name in source_values:
    if worktree_latest.get(name) != value or project_latest.get(f"{project_prefix}/{name}") != value:
        raise SystemExit(f"final source entry failed verification: {name}")
if project_latest.get(project_index_value[1]) != project_index_value[0]:
    raise SystemExit("final worktree-index hash failed verification")

report_text = f"""# CONTINUATION-252 — append-only manifest verification

The first append helper wrote the local evidence hashes and appended the corresponding worktree/project rows. Its final verifier then failed while parsing historical blank lines in `evidence/SHA256SUMS`; the worktree index also contains one older `None` digest row. No index rows were removed or rewritten.

This independent verifier ignored blank and malformed historical rows only for lookup; it preserved them byte-for-byte. Before and after the final append, the original worktree and project manifest prefixes match the saved count/SHA-256 receipt. Latest entries for the local evidence, source files, local index, and referenced worktree index were re-read and matched.

Historical index anomalies observed at verification time: worktree blanks={worktree_blank}, malformed rows={worktree_malformed}; project blanks={project_blank}, malformed rows={project_malformed}. These are retained as historical data and are not treated as valid hashes.
"""
REPORT.write_text(report_text, encoding="utf-8")
report_hash = digest(REPORT.read_bytes())
with LOCAL.open("ab") as stream:
    stream.write(f"{report_hash}  manifest-verification.md\n".encode())
    stream.flush(); os.fsync(stream.fileno())
local_hash = digest(LOCAL.read_bytes())
worktree_report_values = [
    (report_hash, f"{evidence_prefix}/manifest-verification.md"),
    (local_hash, f"{evidence_prefix}/SHA256SUMS"),
]
append(WORKTREE_INDEX, worktree_report_values)
project_report_values = [
    (report_hash, f"{project_prefix}/{evidence_prefix}/manifest-verification.md"),
    (local_hash, f"{project_prefix}/{evidence_prefix}/SHA256SUMS"),
    (digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS"),
]
append(PROJECT_INDEX, project_report_values)

for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = path.read_bytes(); lines = data.splitlines(keepends=True); prior = before[label]
    if digest(b"".join(lines[:prior["count"]])) != prior["sha256"]:
        raise SystemExit(f"original {label} prefix changed at final verification")
local_latest, _, local_malformed = parsed(LOCAL)
worktree_latest, _, _ = parsed(WORKTREE_INDEX)
project_latest, _, _ = parsed(PROJECT_INDEX)
if local_malformed:
    raise SystemExit("local manifest has malformed rows")
for name, value in local_latest.items():
    path = ROOT / name
    if not path.is_file() or digest(path.read_bytes()) != value:
        raise SystemExit(f"local manifest latest value mismatch: {name}")
    if worktree_latest.get(f"{evidence_prefix}/{name}") != value or project_latest.get(f"{project_prefix}/{evidence_prefix}/{name}") != value:
        raise SystemExit(f"global manifest latest value mismatch: {name}")
for value, name in source_values:
    if digest((WORKTREE / name).read_bytes()) != value or worktree_latest.get(name) != value or project_latest.get(f"{project_prefix}/{name}") != value:
        raise SystemExit(f"global source manifest latest value mismatch: {name}")
if project_latest.get(f"{project_prefix}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not match final worktree index hash")
print(json.dumps({
    "status": "verified",
    "local_evidence_entries": len(local_latest),
    "source_entries_verified": len(source_values),
    "original_prefixes_preserved": True,
    "latest_local_worktree_project_hashes_verified": True,
    "historical_blank_malformed_rows_retained": worktree_blank + worktree_malformed + project_blank + project_malformed,
}, sort_keys=True))
