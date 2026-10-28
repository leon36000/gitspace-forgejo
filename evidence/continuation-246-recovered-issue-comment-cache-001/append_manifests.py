#!/usr/bin/env python3
"""Append CONTINUATION-246 evidence while preserving both global prefixes."""
from __future__ import annotations
import hashlib,json,os
from pathlib import Path
ROOT=Path(__file__).resolve().parent
WORKTREE=ROOT.parents[1]
PROJECT=WORKTREE.parents[1]
LOCAL=ROOT/"SHA256SUMS"
WI=WORKTREE/"evidence/SHA256SUMS"
PI=PROJECT/"evidence/SHA256SUMS"
PREFIX=ROOT/"manifest-prefix-before.json"
FILES=('append_manifests.py', 'manifest-prefix-before.json', 'local-run.md', 'commands.txt', 'source-review.txt', 'tool-versions.txt', 'source-sha256.txt', 'test.log', 'test.exit', 'format-check.log', 'format-check.exit', 'license-metadata.log', 'license-metadata.exit', 'license-guard-tests.log', 'license-guard-tests.exit', 'diff-check.log', 'diff-check.exit', 'runner-ast.log', 'runner-ast.exit', 'documentation-whitespace.log', 'documentation-whitespace.exit')
SOURCES=('Cargo.toml', 'Cargo.lock', 'scripts/verify_cargo_license_state.py', 'scripts/test_cargo_license_state.py', 'tests/webhook_receiver_process_e2e.rs', 'scripts/run_webhook_reconcile_cli_e2e.py', 'src/bin/forgejo-reconcile-local.rs', 'src/webhook/reconcile.rs', 'src/webhook/http.rs', 'docs/checkpoint.md', 'docs/refusal-evidence-matrix.md', 'evidence/continuation-246-recovered-issue-comment-cache-001/test.log')
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
for p in (WI,PI):
 if not p.is_file() or p.is_symlink():raise SystemExit(f"global index unavailable: {p}")
if LOCAL.exists() and (LOCAL.is_symlink() or LOCAL.stat().st_size):raise SystemExit("local index is nonempty or symlinked")
LOCAL.touch(exist_ok=True)
current={"local":{"count":len(LOCAL.read_bytes().splitlines()),"sha256":digest(LOCAL.read_bytes())},"worktree":{"count":len(WI.read_bytes().splitlines()),"sha256":digest(WI.read_bytes())},"project":{"count":len(PI.read_bytes().splitlines()),"sha256":digest(PI.read_bytes())}}
if PREFIX.exists() or PREFIX.is_symlink():raise SystemExit("prefix receipt already exists")
PREFIX.write_text(json.dumps(current,sort_keys=True,indent=2)+"\n",encoding="utf-8")
local=[]
for n in FILES:
 p=ROOT/n
 if not p.is_file() or p.is_symlink():raise SystemExit(f"local evidence unavailable: {n}")
 local.append((digest(p.read_bytes()),n))
for n in SOURCES:
 if not (WORKTREE/n).is_file() or (WORKTREE/n).is_symlink():raise SystemExit(f"source unavailable: {n}")
append(LOCAL,local)
local_hash=digest(LOCAL.read_bytes())
wp=ROOT.relative_to(WORKTREE).as_posix();pp=WORKTREE.relative_to(PROJECT).as_posix()
witems=[(v,f"{wp}/{n}") for v,n in local]+[(local_hash,f"{wp}/SHA256SUMS")]
pitems=[(v,f"{pp}/{wp}/{n}") for v,n in local]+[(local_hash,f"{pp}/{wp}/SHA256SUMS")]
for n in SOURCES:
 v=digest((WORKTREE/n).read_bytes());witems.append((v,n));pitems.append((v,f"{pp}/{n}"))
append(WI,witems)
pitems.append((digest(WI.read_bytes()),f"{pp}/evidence/SHA256SUMS"))
append(PI,pitems)
for label,path in (("local",LOCAL),("worktree",WI),("project",PI)):
 data=path.read_bytes();lines=data.splitlines(keepends=True);prior=current[label]
 if digest(b"".join(lines[:prior["count"]]))!=prior["sha256"]:raise SystemExit(f"{label} index prefix changed")
for path,items in ((WI,witems),(PI,pitems)):
 seen=latest(path)
 for v,n in items:
  if seen.get(n)!=v:raise SystemExit(f"latest global hash mismatch: {n}")
print(json.dumps({"local_entries_appended":len(local),"worktree_entries_appended":len(witems),"project_entries_appended":len(pitems),"all_prefixes_preserved":True,"latest_entries_verified":True},sort_keys=True))
