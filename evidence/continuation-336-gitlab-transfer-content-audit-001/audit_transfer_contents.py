#!/usr/bin/env python3
"""Read-only, secret-redacting inventory for the pending GitLab migration."""
from __future__ import annotations
import collections
import hashlib
import json
import re
import subprocess
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORKTREE = HERE.parents[1]
SKIP_PREFIX = HERE.relative_to(WORKTREE).as_posix().rstrip("/") + "/"
KNOWN = {
    "gitlab_pat": re.compile(rb"glpat-[A-Za-z0-9_-]{20,}"),
    "private_key_header": re.compile(rb"-----BEGIN (?:RSA |OPENSSH |EC |DSA )?PRIVATE KEY-----"),
    "credential_url": re.compile(rb"https?://[^/\s:@]+:[^/\s@]+@"),
    "github_pat": re.compile(rb"gh[pousr]_[A-Za-z0-9_]{30,}"),
    "openai_key_shape": re.compile(rb"sk-(?:proj-)?[A-Za-z0-9_-]{30,}"),
    "aws_key_shape": re.compile(rb"AKIA[0-9A-Z]{16}"),
}
LITERAL_ASSIGNMENT = re.compile(
    rb"(?i)(?:password|passwd|client_secret|api[_-]?key|access[_-]?token)\s*[:=]\s*[\"']([^\"']{16,})[\"']"
)


def git_paths(*args: str) -> set[str]:
    raw = subprocess.check_output(["git", *args, "-z"], cwd=WORKTREE)
    return {p.decode("utf-8", "surrogateescape") for p in raw.split(b"\0") if p}

changed = git_paths("diff", "--name-only", "--diff-filter=ACMRT", "HEAD")
untracked = git_paths("ls-files", "--others", "--exclude-standard")
untracked_before_audit = {p for p in untracked if not p.startswith(SKIP_PREFIX)}
paths = sorted(changed | untracked_before_audit)
top = collections.Counter()
extensions = collections.Counter()
plain_hits = collections.Counter()
quoted_literal_candidates: set[tuple[str, int]] = set()
binary_count = 0
large = []
zip_files = []
bytecode_files = []
credential_like_names = []
symlinks = []
nonfile_paths = []
for rel in paths:
    path = WORKTREE / rel
    try:
        st = path.lstat()
    except OSError:
        continue
    if path.is_symlink():
        symlinks.append(rel)
        continue
    if not path.is_file():
        nonfile_paths.append(rel)
        continue
    top[rel.split("/", 1)[0]] += 1
    extensions[path.suffix.lower() or "[no extension]"] += 1
    if st.st_size > 20 * 1024 * 1024:
        large.append({"path": rel, "bytes": st.st_size})
    if path.suffix.lower() == ".zip":
        zip_files.append({"path": rel, "bytes": st.st_size})
    if path.suffix.lower() in {".pyc", ".pyo"}:
        bytecode_files.append(rel)
    lname = path.name.lower()
    if lname in {".env", ".env.local", ".env.production", "credentials.json", "secrets.json", "id_rsa", "id_ed25519"}:
        credential_like_names.append(rel)
    try:
        data = path.read_bytes()
    except OSError:
        continue
    if b"\0" in data[:8192]:
        binary_count += 1
    if path.suffix.lower() not in {".pyc", ".pyo"}:
        for kind, pattern in KNOWN.items():
            if pattern.search(data):
                plain_hits[kind] += 1
    for no, line in enumerate(data.splitlines(), 1):
        if LITERAL_ASSIGNMENT.search(line):
            quoted_literal_candidates.add((rel, no))

zip_entries = 0
zip_hits = collections.Counter()
zip_named_credentials = []
zip_errors = []
for item in zip_files:
    path = WORKTREE / item["path"]
    try:
        with zipfile.ZipFile(path) as archive:
            for info in archive.infolist():
                if info.is_dir():
                    continue
                zip_entries += 1
                lower = info.filename.lower()
                if any(part in lower for part in (".env", "credentials", "secret", "token", "private_key", "id_rsa", "id_ed25519")):
                    zip_named_credentials.append({"archive": item["path"], "entry": info.filename})
                if info.file_size > 20 * 1024 * 1024:
                    continue
                content = archive.read(info)
                for kind, pattern in KNOWN.items():
                    if pattern.search(content):
                        zip_hits[kind] += 1
    except Exception as exc:
        zip_errors.append({"archive": item["path"], "error_type": type(exc).__name__})

manifest = (WORKTREE / "evidence/SHA256SUMS").read_text(encoding="utf-8")
manifest_lines = [
    line.partition("  ")[2]
    for line in manifest.splitlines()
    if "  " in line
]
manifest_paths = {
    path for path in manifest_lines
}
manifest_counts = {
    "pyc_path_entries": sum(path.endswith((".pyc", ".pyo")) for path in manifest_lines),
    "unique_pyc_paths": sum(path.endswith((".pyc", ".pyo")) for path in manifest_paths),
    "zip_path_entries": sum(path.endswith(".zip") for path in manifest_lines),
    "unique_zip_paths": sum(path.endswith(".zip") for path in manifest_paths),
    "direct_bytecode_files_referenced": sum(path in manifest_paths for path in bytecode_files),
    "direct_bytecode_files_not_referenced": sum(path not in manifest_paths for path in bytecode_files),
    "zip_files_referenced": sum(item["path"] in manifest_paths for item in zip_files),
    "zip_files_not_referenced": sum(item["path"] not in manifest_paths for item in zip_files),
}
nested_repo = WORKTREE / "evidence/forgejo-v17-lfs-reflog-witness-poc-065/scratch/server.git"
bundle = HERE / "forgejo-v17-lfs-reflog-witness-server.bundle"
bundle_bytes = bundle.read_bytes()
bundle_heads = subprocess.check_output(
    ["git", "bundle", "list-heads", str(bundle)], cwd=WORKTREE, text=True
).splitlines()
nested_worktree = WORKTREE / "evidence/forgejo-v17-lfs-reflog-witness-poc-065/scratch/work"
nested_worktree_status = subprocess.check_output(
    ["git", "-C", str(nested_worktree), "status", "--porcelain"], cwd=WORKTREE, text=True
)
nested_worktree_head = subprocess.check_output(
    ["git", "-C", str(nested_worktree), "rev-parse", "HEAD"], cwd=WORKTREE, text=True
).strip()
try:
    evidence_bytes = sum(
        p.stat().st_size
        for p in (WORKTREE / "evidence").rglob("*")
        if p.is_file() and not p.is_symlink() and HERE not in p.parents
    )
except OSError:
    evidence_bytes = None
result = {
    "base_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=WORKTREE, text=True).strip(),
    "tracked_changed_files": len(changed),
    "untracked_nonignored_path_entries_before_audit": len(untracked_before_audit),
    "unique_candidate_path_count": len(paths),
    "unique_candidate_file_count": len(paths) - len(nonfile_paths),
    "nonfile_candidate_paths": nonfile_paths,
    "candidate_files_by_top_level": dict(sorted(top.items())),
    "candidate_extensions_top_12": extensions.most_common(12),
    "evidence_tree_bytes": evidence_bytes,
    "zip_count": len(zip_files),
    "zip_total_bytes": sum(i["bytes"] for i in zip_files),
    "zip_uncompressed_file_entries": zip_entries,
    "direct_bytecode_cache_count": len(bytecode_files),
    "root_manifest_refs_to_bytecode_or_archives": manifest_counts,
    "binary_file_count": binary_count,
    "files_over_20_mib": large,
    "symlink_paths": symlinks,
    "credential_named_paths": credential_like_names,
    "known_secret_format_file_hits": dict(plain_hits),
    "known_secret_format_zip_entry_hits": dict(zip_hits),
    "zip_entries_with_credential_like_names": zip_named_credentials,
    "quoted_credential_literal_candidates": sorted(
        ({"path": p, "line": n} for p, n in quoted_literal_candidates),
        key=lambda x: (x["path"], x["line"]),
    ),
    "zip_read_errors": zip_errors,
    "nested_bare_git_repository": {
        "path": nested_repo.relative_to(WORKTREE).as_posix(),
        "bare": subprocess.check_output(
            ["git", "--git-dir", str(nested_repo), "rev-parse", "--is-bare-repository"],
            cwd=WORKTREE,
            text=True,
        ).strip()
        == "true",
        "transfer_bundle": bundle.relative_to(WORKTREE).as_posix(),
        "bundle_bytes": len(bundle_bytes),
        "bundle_sha256": hashlib.sha256(bundle_bytes).hexdigest(),
        "bundle_refs": bundle_heads,
    },
    "nested_git_worktree": {
        "path": nested_worktree.relative_to(WORKTREE).as_posix(),
        "head": nested_worktree_head,
        "clean": not nested_worktree_status,
        "same_head_as_bundle": bool(bundle_heads)
        and nested_worktree_head == bundle_heads[0].split()[0],
    },
    "excluded_local_instruction_file": ".claude/ralph-loop.local.md",
    "remote_auth_or_git_exchange_attempted": False,
}
(HERE / "transfer-content-audit.json").write_text(json.dumps(result, sort_keys=True, indent=2) + "\n", encoding="utf-8")
print(json.dumps(result, sort_keys=True, indent=2))
