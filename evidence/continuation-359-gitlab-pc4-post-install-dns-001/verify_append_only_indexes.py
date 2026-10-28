#!/usr/bin/env python3
"""Verify CONTINUATION-359 hashes and append-only index prefixes."""
import hashlib, json
from pathlib import Path
ROOT=Path(__file__).resolve().parent; WORKTREE=ROOT.parents[1]; PROJECT=WORKTREE.parents[1]
PREFIX=json.loads((ROOT/"manifest-prefix-before.json").read_text())
LOCAL=ROOT/"SHA256SUMS"; WI=WORKTREE/"evidence/SHA256SUMS"; PI=PROJECT/"evidence/SHA256SUMS"
DOCS=("docs/checkpoint.md","docs/implementation-plan.md"); EXCLUDED={"SHA256SUMS","manifest-check.txt","global-manifest-check.txt"}
def digest(data): return hashlib.sha256(data).hexdigest()
def latest(path):
    out={}
    for line in path.read_text().splitlines():
        value, sep, name=line.partition("  ")
        if sep: out[name]=value
    return out
for key,path in (("worktree",WI),("project",PI)):
    lines=path.read_bytes().splitlines(keepends=True); expected=PREFIX[key]
    if len(lines)<expected["line_count"] or digest(b"".join(lines[:expected["line_count"]]))!=expected["sha256"]: raise SystemExit(f"{key} append-only prefix mismatch")
actual={}
for path in ROOT.rglob("*"):
    if "__pycache__" in path.parts: continue
    if path.is_symlink(): raise SystemExit(f"symlink: {path.relative_to(ROOT)}")
    if path.is_file() and path.name not in EXCLUDED: actual[path.relative_to(ROOT).as_posix()]=digest(path.read_bytes())
if actual!=latest(LOCAL): raise SystemExit("local evidence index incomplete/stale")
work=latest(WI); project=latest(PI)
for name,value in actual.items():
    wp=f"evidence/{ROOT.name}/{name}"; pp=f".runtime/webhook-005-workspace/{wp}"
    if work.get(wp)!=value or project.get(pp)!=value: raise SystemExit(f"global hash mismatch: {name}")
ld=digest(LOCAL.read_bytes())
if work.get(f"evidence/{ROOT.name}/SHA256SUMS")!=ld or project.get(f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS")!=ld: raise SystemExit("global manifest does not bind local manifest")
for name in DOCS:
    value=digest((WORKTREE/name).read_bytes())
    if work.get(name)!=value or project.get(f".runtime/webhook-005-workspace/{name}")!=value: raise SystemExit(f"documentation hash mismatch: {name}")
if project.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS")!=digest(WI.read_bytes()): raise SystemExit("parent does not bind current worktree index")
print(json.dumps({"local_files_verified":len(actual),"worktree_prefix_preserved":True,"project_prefix_preserved":True,"parent_binds_worktree_index":True},sort_keys=True))
