#!/usr/bin/env python3
"""Append CONTINUATION-359 to local, worktree, and project indexes."""
import hashlib, json
from pathlib import Path
ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
DOCS = ("docs/checkpoint.md", "docs/implementation-plan.md")
EXCLUDED = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
def digest(data): return hashlib.sha256(data).hexdigest()
def append(path, entries):
    with path.open("ab") as stream:
        for value, name in entries: stream.write(f"{value}  {name}\n".encode())
if LOCAL.exists() or LOCAL.is_symlink(): raise SystemExit("local index already exists")
for path in (WORKTREE_INDEX, PROJECT_INDEX):
    if not path.is_file() or path.is_symlink(): raise SystemExit(f"index unavailable: {path}")
prefix = {}
for key, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = path.read_bytes(); prefix[key] = {"line_count": len(data.splitlines()), "sha256": digest(data)}
(ROOT / "manifest-prefix-before.json").write_text(json.dumps(prefix, sort_keys=True, indent=2)+"\n")
files = []
for path in ROOT.rglob("*"):
    if "__pycache__" in path.parts: continue
    if path.is_symlink(): raise SystemExit(f"symlink: {path.relative_to(ROOT)}")
    if path.is_file() and path.name not in EXCLUDED: files.append(path)
files.sort(key=lambda p: p.relative_to(ROOT).as_posix())
entries = [(digest(p.read_bytes()), p.relative_to(ROOT).as_posix()) for p in files]
LOCAL.write_text("".join(f"{value}  {name}\n" for value,name in entries))
ld = digest(LOCAL.read_bytes())
work = [(value, f"evidence/{ROOT.name}/{name}") for value,name in entries] + [(ld, f"evidence/{ROOT.name}/SHA256SUMS")]
project = [(value, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/{name}") for value,name in entries] + [(ld, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS")]
for relative in DOCS:
    value = digest((WORKTREE / relative).read_bytes())
    work.append((value, relative)); project.append((value, f".runtime/webhook-005-workspace/{relative}"))
append(WORKTREE_INDEX, work); append(PROJECT_INDEX, project)
append(PROJECT_INDEX, [(digest(WORKTREE_INDEX.read_bytes()), ".runtime/webhook-005-workspace/evidence/SHA256SUMS")])
print(json.dumps({"local_evidence_files":len(entries),"worktree_entries":len(work),"project_entries":len(project)+1},sort_keys=True))
