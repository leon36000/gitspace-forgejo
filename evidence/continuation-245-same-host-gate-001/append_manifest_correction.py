#!/usr/bin/env python3
"""Append a corrected latest source manifest without rewriting prior entries."""
from __future__ import annotations
import hashlib,json,os
from pathlib import Path
ROOT=Path(__file__).resolve().parent
WORKTREE=ROOT.parents[1]
PROJECT=WORKTREE.parents[1]
LOCAL=ROOT/"SHA256SUMS"
WI=WORKTREE/"evidence/SHA256SUMS"
PI=PROJECT/"evidence/SHA256SUMS"
PREFIX=ROOT/"manifest-correction-prefix.json"
LOCAL_FILES=("append_manifest_correction.py","manifest-correction-prefix.json","manifest-correction.md","source-sha256-latest.txt")
def digest(data:bytes)->str:return hashlib.sha256(data).hexdigest()
def latest(path:Path)->dict[str,str]:
 out={}
 for raw in path.read_bytes().splitlines():
  try:v,n=raw.decode().split("  ",1)
  except (UnicodeDecodeError,ValueError):continue
  if len(v)==64 and n:out[n]=v
 return out
def append(path:Path,items:list[tuple[str,str]])->None:
 with path.open("ab") as f:
  for v,n in items:f.write(f"{v}  {n}\n".encode())
  f.flush();os.fsync(f.fileno())
for p in (LOCAL,WI,PI):
 if not p.is_file() or p.is_symlink():raise SystemExit(f"manifest unavailable: {p}")
if PREFIX.exists() or PREFIX.is_symlink():raise SystemExit("correction prefix already exists")
current={"local":{"count":len(LOCAL.read_bytes().splitlines()),"sha256":digest(LOCAL.read_bytes())},"worktree":{"count":len(WI.read_bytes().splitlines()),"sha256":digest(WI.read_bytes())},"project":{"count":len(PI.read_bytes().splitlines()),"sha256":digest(PI.read_bytes())}}
PREFIX.write_text(json.dumps(current,sort_keys=True,indent=2)+"\n",encoding="utf-8")
local=[]
for n in LOCAL_FILES:
 p=ROOT/n
 if not p.is_file() or p.is_symlink():raise SystemExit(f"missing correction evidence: {n}")
 local.append((digest(p.read_bytes()),n))
append(LOCAL,local)
local_hash=digest(LOCAL.read_bytes())
wp=ROOT.relative_to(WORKTREE).as_posix();pp=WORKTREE.relative_to(PROJECT).as_posix()
witems=[(v,f"{wp}/{n}") for v,n in local]+[(local_hash,f"{wp}/SHA256SUMS")]
pitems=[(v,f"{pp}/{wp}/{n}") for v,n in local]+[(local_hash,f"{pp}/{wp}/SHA256SUMS")]
for raw in (ROOT/"source-sha256-latest.txt").read_text(encoding="utf-8").splitlines():
 v,n=raw.split("  ",1)
 witems.append((v,n));pitems.append((v,f"{pp}/{n}"))
append(WI,witems)
pitems.append((digest(WI.read_bytes()),f"{pp}/evidence/SHA256SUMS"))
append(PI,pitems)
for label,path in (("local",LOCAL),("worktree",WI),("project",PI)):
 data=path.read_bytes();lines=data.splitlines(keepends=True);before=current[label]
 if digest(b"".join(lines[:before["count"]]))!=before["sha256"]:raise SystemExit(f"{label} prefix mismatch")
for path,items in ((WI,witems),(PI,pitems)):
 seen=latest(path)
 for v,n in items:
  if seen.get(n)!=v:raise SystemExit(f"latest index mismatch: {n}")
print(json.dumps({"local_entries_appended":len(local),"worktree_entries_appended":len(witems),"project_entries_appended":len(pitems),"all_existing_prefixes_preserved":True,"latest_entries_verified":True},sort_keys=True))
