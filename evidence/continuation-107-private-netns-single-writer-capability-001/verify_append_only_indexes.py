#!/usr/bin/env python3
"""Verify this proof set and the append-only worktree/project indexes."""
from __future__ import annotations
import hashlib,json
from pathlib import Path
ROOT=Path(__file__).resolve().parent
WORKTREE=ROOT.parents[1]
PROJECT=WORKTREE.parents[1]
W=WORKTREE/'evidence/SHA256SUMS'; P=PROJECT/'evidence/SHA256SUMS'
PREFIX=json.loads((ROOT/'manifest-prefix-before.json').read_text())
def sha(path:Path)->str: return hashlib.sha256(path.read_bytes()).hexdigest()
def sha_bytes(data:bytes)->str: return hashlib.sha256(data).hexdigest()
def latest(path:Path)->dict[str,str]:
 out={}
 for line in path.read_text().splitlines():
  bits=line.split('  ',1)
  if len(bits)==2: out[bits[1]]=bits[0]
 return out
for path,name in ((W,'worktree'),(P,'project')):
 lines=path.read_bytes().splitlines(keepends=True); n=PREFIX[name]['line_count_before_append']
 if sha_bytes(b''.join(lines[:n]))!=PREFIX[name]['sha256_before_append']: raise ValueError(f'{name} prefix changed')
w_latest=latest(W); p_latest=latest(P)
files=sorted(x for x in ROOT.rglob('*') if x.is_file())
for path in files:
 rel=path.relative_to(WORKTREE).as_posix(); project_rel=(Path('.runtime/webhook-005-workspace')/rel).as_posix(); digest=sha(path)
 if w_latest.get(rel)!=digest or p_latest.get(project_rel)!=digest: raise ValueError(f'global index mismatch: {rel}')
count=0
for line in (ROOT/'SHA256SUMS').read_text().splitlines():
 expected,rel=line.split('  ',1); path=(ROOT/rel).resolve(strict=True)
 if ROOT.resolve() not in path.parents or sha(path)!=expected: raise ValueError(f'local manifest mismatch: {rel}')
 count+=1
for rel in ('docs/architecture.md','docs/checkpoint.md'):
 path=WORKTREE/rel; project_rel=(Path('.runtime/webhook-005-workspace')/rel).as_posix()
 if w_latest.get(rel)!=sha(path) or p_latest.get(project_rel)!=sha(path): raise ValueError(f'doc index mismatch: {rel}')
if p_latest.get('.runtime/webhook-005-workspace/evidence/SHA256SUMS')!=sha(W): raise ValueError('project index does not track worktree index')
print(json.dumps({'worktree_prefix_preserved':True,'project_prefix_preserved':True,'local_entries_verified':count,'artifact_files_verified':len(files),'docs_verified':2,'indexes_current':True},sort_keys=True))
