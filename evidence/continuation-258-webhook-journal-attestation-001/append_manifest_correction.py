#!/usr/bin/env python3
"""Append corrected evidence hashes while preserving every existing manifest row."""
from __future__ import annotations
import hashlib, json, os
from pathlib import Path
ROOT=Path(__file__).resolve().parent
WORKTREE=ROOT.parents[1]
PROJECT=WORKTREE.parents[1]
LOCAL=ROOT/"SHA256SUMS"
PREFIX=json.loads((ROOT/"manifest-prefix-before.json").read_text(encoding="utf-8"))
WT=WORKTREE/"evidence/SHA256SUMS"
PR=PROJECT/"evidence/SHA256SUMS"
NAMES=["append_manifest_correction.py", "checks-summary.json", "commands.txt", "local-run.md", "manifest-verification-final.json", "verify_manifests.py"]
def h(data: bytes)->str: return hashlib.sha256(data).hexdigest()
def append(path: Path, rows: list[tuple[str,str]])->None:
    with path.open("ab") as f:
        for value,name in rows: f.write(f"{value}  {name}\n".encode())
        f.flush(); os.fsync(f.fileno())
def verify_prefix(path: Path, label: str)->None:
    lines=path.read_bytes().splitlines(keepends=True); count=PREFIX[label]["line_count"]
    if len(lines)<count or h(b"".join(lines[:count]))!=PREFIX[label]["sha256"]: raise SystemExit(f"{label} historical prefix changed")
for name in NAMES:
    q=ROOT/name
    if not q.is_file() or q.is_symlink(): raise SystemExit(f"missing correction evidence: {name}")
rows=[(h((ROOT/name).read_bytes()), name) for name in NAMES]
append(LOCAL,rows)
local_hash=h(LOCAL.read_bytes())
prefix=ROOT.relative_to(WORKTREE).as_posix(); project_prefix=WORKTREE.relative_to(PROJECT).as_posix()
wt_rows=[(value,f"{prefix}/{name}") for value,name in rows]+[(local_hash,f"{prefix}/SHA256SUMS")]
append(WT,wt_rows)
pr_rows=[(value,f"{project_prefix}/{prefix}/{name}") for value,name in rows]
pr_rows.append((local_hash,f"{project_prefix}/{prefix}/SHA256SUMS"))
pr_rows.append((h(WT.read_bytes()),f"{project_prefix}/evidence/SHA256SUMS"))
append(PR,pr_rows)
verify_prefix(WT,"worktree"); verify_prefix(PR,"project")
print(json.dumps({"status":"correction_appended","files_rehashed":len(rows),"worktree_rows_appended":len(wt_rows),"project_rows_appended":len(pr_rows),"historical_prefixes_preserved":True},sort_keys=True))
