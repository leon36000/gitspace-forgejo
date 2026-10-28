#!/usr/bin/env python3
"""Verify the local evidence set and append-only parent/worktree indexes."""
from __future__ import annotations
import hashlib, json
from pathlib import Path
ROOT=Path(__file__).resolve().parent
WORKTREE=ROOT.parents[1]
PROJECT=WORKTREE.parents[1]
WORKTREE_INDEX=WORKTREE/'evidence/SHA256SUMS'
PROJECT_INDEX=PROJECT/'evidence/SHA256SUMS'
PREFIX=json.loads((ROOT/'manifest-prefix-before.json').read_text())
def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()
def latest(path: Path) -> dict[str,str]:
    result={}
    for line in path.read_text().splitlines():
        parts=line.split('  ',1)
        if len(parts)==2: result[parts[1]]=parts[0]
    return result
def check_prefix(path: Path,name: str) -> None:
    lines=path.read_bytes().splitlines(keepends=True)
    count=PREFIX[name]['line_count_before_append']
    if hashlib.sha256(b''.join(lines[:count])).hexdigest()!=PREFIX[name]['sha256_before_append']:
        raise ValueError(f'{name} append-only prefix changed')
for path,name in ((WORKTREE_INDEX,'worktree'),(PROJECT_INDEX,'project')): check_prefix(path,name)
local_latest=latest(WORKTREE_INDEX)
project_latest=latest(PROJECT_INDEX)
files=sorted(p for p in ROOT.rglob('*') if p.is_file())
for path in files:
    rel=path.relative_to(WORKTREE).as_posix()
    project_rel=(Path('.runtime/webhook-005-workspace')/rel).as_posix()
    observed=digest(path)
    if local_latest.get(rel)!=observed or project_latest.get(project_rel)!=observed:
        raise ValueError(f'global index mismatch: {rel}')
manifest=ROOT/'SHA256SUMS'
entries=0
for line in manifest.read_text().splitlines():
    expected,rel=line.split('  ',1)
    path=(ROOT/rel).resolve(strict=True)
    if ROOT.resolve() not in path.parents or digest(path)!=expected:
        raise ValueError(f'local manifest mismatch: {rel}')
    entries+=1
for rel in ('src/forgejo.rs','src/bin/forgejo-reconcile-local.rs','tests/forgejo_merge_writer_admission.rs','docs/architecture.md','docs/checkpoint.md','docs/refusal-evidence-matrix.md'):
    path=WORKTREE/rel
    project_rel=(Path('.runtime/webhook-005-workspace')/rel).as_posix()
    if local_latest.get(rel)!=digest(path) or project_latest.get(project_rel)!=digest(path):
        raise ValueError(f'source index mismatch: {rel}')
if project_latest.get('.runtime/webhook-005-workspace/evidence/SHA256SUMS')!=digest(WORKTREE_INDEX):
    raise ValueError('project index does not track the current worktree index')
print(json.dumps({'worktree_prefix_preserved':True,'project_prefix_preserved':True,'local_evidence_entries_verified':entries,'local_evidence_files_verified':len(files),'source_and_docs_verified':6,'worktree_and_project_indexes_current':True},sort_keys=True))
