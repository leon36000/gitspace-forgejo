#!/usr/bin/env python3
"""Audit modified/untracked text evidence against a saved GitLab tree; never uploads."""
from __future__ import annotations
import argparse, hashlib, json, re, subprocess
from collections import Counter
from pathlib import Path, PurePosixPath

EXCLUDED_PATHS={"docs/checkpoint.md","evidence/SHA256SUMS"}
EXCLUDED_DIRS={"gitlab-pc4-transfer-006",".git",".runtime",".claude","target",".venv","__pycache__",".pytest_cache",".mypy_cache",".ruff_cache"}
BINARY_EXTS={".zip",".bundle",".b64",".asc",".db",".sqlite",".sqlite3",".png",".jpg",".jpeg",".pdf",".so",".rlib",".crate",".bin",".exe",".dll",".msi",".tar",".gz",".zst",".xz",".7z",".docx",".mp4",".mp3",".patch",".diff"}
THIRD_PARTY_SOURCE_EXTS={".go",".rs",".c",".h",".proto"}
SECRET_PATTERNS={
 "gitlab_pat": re.compile(rb"glpat-[A-Za-z0-9_-]{20,}"),
 "github_token": re.compile(rb"(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})"),
 "aws_access_key": re.compile(rb"AKIA[0-9A-Z]{16}"),
 "private_key": re.compile(rb"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----"),
 "bearer": re.compile(rb"Bearer\s+[A-Za-z0-9._~+/-]{32,}", re.I),
 "credential_assignment": re.compile(rb"(?:password|passwd|client_secret|api[_-]?key|access[_-]?token)\s*[:=]\s*[\"'][^\"']{12,}[\"']", re.I),
}

def git_blob(data: bytes) -> str:
    return hashlib.sha1(b"blob " + str(len(data)).encode() + bytes([0]) + data).hexdigest()

def main() -> int:
    ap=argparse.ArgumentParser()
    ap.add_argument("--root",type=Path,default=Path.cwd())
    ap.add_argument("--remote-tree",type=Path,required=True)
    ap.add_argument("--output",type=Path,required=True)
    args=ap.parse_args()
    root=args.root.resolve(strict=True)
    tree=json.loads(args.remote_tree.read_text(encoding="utf-8"))
    remote={x["path"]:x for x in tree if x.get("type")=="blob"}
    status=subprocess.run(["git","status","--porcelain","-uall"],cwd=root,check=True,text=True,capture_output=True).stdout.splitlines()
    counts=Counter(status_entries=len(status))
    patterns=Counter()
    candidates=[]
    candidate_bytes=0
    for line in status:
        rel=line[3:].split(" -> ")[-1]
        if rel in EXCLUDED_PATHS:
            counts["bookkeeping_file_excluded"]+=1
            continue
        pure=PurePosixPath(rel)
        if any(part in EXCLUDED_DIRS for part in pure.parts):
            counts["state_directory_excluded"]+=1
            continue
        path=root/rel
        try:
            if not path.is_file():
                counts["non_regular"]+=1
                continue
            counts["regular_files"]+=1
            ext=path.suffix.lower()
            if ext in BINARY_EXTS or path.name==".gitconfig":
                counts["binary_or_archive_excluded"]+=1
                continue
            data=path.read_bytes()
            if bytes([0]) in data:
                counts["nul_or_binary_excluded"]+=1
                continue
            try:
                data.decode("utf-8")
            except UnicodeDecodeError:
                counts["invalid_utf8_excluded"]+=1
                continue
            found=[name for name,rx in SECRET_PATTERNS.items() if rx.search(data)]
            if found:
                counts["known_secret_pattern_excluded"]+=1
                for name in found: patterns[name]+=1
                continue
            if rel.startswith("evidence/") and ext in THIRD_PARTY_SOURCE_EXTS:
                counts["third_party_source_snapshot_excluded"]+=1
                continue
            counts["text_candidates"]+=1
            blob=git_blob(data)
            remote_entry=remote.get(rel)
            if remote_entry and remote_entry.get("id")==blob:
                counts["remote_identical"]+=1
                continue
            if remote_entry:
                counts["remote_divergent"]+=1
            else:
                counts["remote_missing"]+=1
            candidate_bytes+=len(data)
            candidates.append({"path":rel,"git_blob_sha1":blob,"mode":"100755" if path.stat().st_mode & 0o111 else "100644","size_bytes":len(data)})
        except OSError:
            counts["unreadable"]+=1
    candidates.sort(key=lambda x:x["path"])
    args.output.mkdir(parents=True,exist_ok=True)
    with (args.output/"candidate-manifest.jsonl").open("w",encoding="utf-8",newline="\n") as out:
        for row in candidates:
            out.write(json.dumps(row,ensure_ascii=False,separators=(",",":"))+"\n")
    summary={
        "remote_tree_entries":len(tree),"remote_blob_count":len(remote),
        "counters":dict(counts),"known_secret_pattern_counts":dict(patterns),
        "candidate_bytes":candidate_bytes,"candidate_paths":len(candidates),
        "candidate_top_level_counts":dict(Counter(PurePosixPath(x["path"]).parts[0] for x in candidates)),
        "all_candidates_are_evidence":all(x["path"].startswith("evidence/") for x in candidates),
        "limitations":["heuristic filename/extension exclusions","known patterns only; not exhaustive","UTF-8 evidence can still contain private or third-party excerpts","inventory is a worktree status snapshot"],
    }
    (args.output/"audit-summary.json").write_text(json.dumps(summary,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(summary,ensure_ascii=False,sort_keys=True))
    return 0

if __name__=="__main__":
    raise SystemExit(main())
