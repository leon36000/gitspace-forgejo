#!/usr/bin/env python3
"""Append CONTINUATION-245 evidence without rewriting history."""
from __future__ import annotations
import hashlib,json,os
from pathlib import Path
ROOT=Path(__file__).resolve().parent
WORKTREE=ROOT.parents[1]
PROJECT=WORKTREE.parents[1]
LOCAL=ROOT/"SHA256SUMS"
WORKTREE_INDEX=WORKTREE/"evidence/SHA256SUMS"
PROJECT_INDEX=PROJECT/"evidence/SHA256SUMS"
PREFIX=ROOT/"manifest-prefix-before.json"
FILES=('append_manifests.py', 'manifest-prefix-before.json', 'local-run.md', 'commands.txt', 'source-review.txt', 'tool-versions.txt', 'source-sha256.txt', 'successful-composition.log', 'successful-composition.command.txt', 'successful-composition.exit', 'preflight-refusal-001.log', 'preflight-refusal-001.command.txt', 'preflight-refusal-001.exit', 'preflight-source-snapshot.zip', 'format-check.log', 'format-check.exit', 'diff-check.log', 'diff-check.exit', 'documentation-whitespace.log', 'documentation-whitespace.exit')
SOURCES=('scripts/run_mission_012_live_composed_e2e.py', 'scripts/systemd_process_scope.py', 'scripts/mission_012_egress_barrier_mcp_host.py', 'scripts/mission_012_mcp_supervisor.py', 'scripts/mission_012_sandboxed_test_adapter.py', 'scripts/mission_012_test_agent_adapter.py', 'tests/test_mission_012_live_composed_e2e.py', 'tests/test_mission_012_egress_barrier_mcp_host.py', 'tests/fixtures/mission-012/requirements/MISSION-012-CLAMP-001.json', 'tests/fixtures/mission-012/policy/MISSION-012-CLAMP-001-v4.json', 'docs/checkpoint.md', 'docs/refusal-evidence-matrix.md', 'evidence/mission-012-live-agent-webhook-cont245-gate-002/SHA256SUMS', 'evidence/mission-012-live-agent-webhook-cont245-gate-002/composed-run.json', 'evidence/mission-012-live-agent-webhook-cont245-gate-002/composed-stage-summaries.json', 'evidence/mission-012-live-agent-webhook-cont245-gate-002/same-host-gate-verification.json', 'evidence/mission-012-live-agent-webhook-cont245-gate-002/independent-verifier.json', 'evidence/mission-012-live-agent-webhook-cont245-gate-002/forgejo-result.json', 'evidence/mission-012-live-agent-webhook-cont245-gate-002/postmerge-verification.json', 'evidence/mission-012-live-agent-webhook-cont245-gate-002/pr-webhook-reconciliation.json', 'evidence/mission-012-live-agent-webhook-cont245-gate-002/composition-source-snapshot.zip')
def digest(data:bytes)->str:return hashlib.sha256(data).hexdigest()
def entries(path:Path)->dict[str,str]:
 out={}
 for raw in path.read_bytes().splitlines():
  try:value,name=raw.decode().split("  ",1)
  except (UnicodeDecodeError,ValueError):continue
  if len(value)==64 and name:out[name]=value
 return out
def append(path:Path,items:list[tuple[str,str]])->None:
 with path.open("ab") as f:
  for value,name in items:f.write(f"{value}  {name}\n".encode())
  f.flush();os.fsync(f.fileno())
for p in (WORKTREE_INDEX,PROJECT_INDEX):
 if not p.is_file() or p.is_symlink():raise SystemExit(f"global index unavailable: {p}")
if LOCAL.exists() and (LOCAL.is_symlink() or LOCAL.stat().st_size):raise SystemExit("local index already populated; refusing overwrite")
LOCAL.touch(exist_ok=True)
current={"local":{"line_count":0,"sha256":digest(b"")},"worktree":{"line_count":len(WORKTREE_INDEX.read_bytes().splitlines()),"sha256":digest(WORKTREE_INDEX.read_bytes())},"project":{"line_count":len(PROJECT_INDEX.read_bytes().splitlines()),"sha256":digest(PROJECT_INDEX.read_bytes())}}
if PREFIX.exists() or PREFIX.is_symlink():raise SystemExit("prefix receipt already exists")
PREFIX.write_text(json.dumps(current,sort_keys=True,indent=2)+"\n",encoding="utf-8")
local=[]
for name in FILES:
 p=ROOT/name
 if not p.is_file() or p.is_symlink():raise SystemExit(f"local evidence unavailable: {name}")
 local.append((digest(p.read_bytes()),name))
for name in SOURCES:
 p=WORKTREE/name
 if not p.is_file() or p.is_symlink():raise SystemExit(f"source evidence unavailable: {name}")
append(LOCAL,local)
local_hash=digest(LOCAL.read_bytes())
wp=ROOT.relative_to(WORKTREE).as_posix();pp=WORKTREE.relative_to(PROJECT).as_posix()
witems=[(v,f"{wp}/{n}") for v,n in local]+[(local_hash,f"{wp}/SHA256SUMS")]
pitems=[(v,f"{pp}/{wp}/{n}") for v,n in local]+[(local_hash,f"{pp}/{wp}/SHA256SUMS")]
for name in SOURCES:
 value=digest((WORKTREE/name).read_bytes());witems.append((value,name));pitems.append((value,f"{pp}/{name}"))
append(WORKTREE_INDEX,witems)
pitems.append((digest(WORKTREE_INDEX.read_bytes()),f"{pp}/evidence/SHA256SUMS"))
append(PROJECT_INDEX,pitems)
for label,path in (("local",LOCAL),("worktree",WORKTREE_INDEX),("project",PROJECT_INDEX)):
 data=path.read_bytes();lines=data.splitlines(keepends=True);before=current[label]
 if digest(b"".join(lines[:before["line_count"]]))!=before["sha256"]:raise SystemExit(f"{label} index prefix changed")
for path,items in ((WORKTREE_INDEX,witems),(PROJECT_INDEX,pitems)):
 latest=entries(path)
 for value,name in items:
  if latest.get(name)!=value:raise SystemExit(f"global latest mismatch: {name}")
print(json.dumps({"local_entries_appended":len(local),"worktree_entries_appended":len(witems),"project_entries_appended":len(pitems),"all_prefixes_preserved":True,"latest_global_entries_match":True},sort_keys=True))
