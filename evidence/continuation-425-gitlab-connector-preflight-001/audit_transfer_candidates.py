#!/usr/bin/env python3
"""Audit GitLab transfer candidates without reading local agent session state."""

from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
AUDIT_OUTPUT = "evidence/continuation-425-gitlab-connector-preflight-001/export-scan.json"
CONTROL_MANIFESTS = {
    "evidence/SHA256SUMS",
    "evidence/continuation-425-gitlab-connector-preflight-001/transfer-manifest.sha256",
}
PATTERNS = {
    "gitlab_pat": re.compile(rb"(?i)\bglpat-[A-Za-z0-9_-]{20,}"),
    "gitlab_runner_token": re.compile(rb"(?i)\bglrt-[A-Za-z0-9_-]{20,}"),
    "github_token": re.compile(rb"(?i)\bgh[pousr]_[A-Za-z0-9]{20,}"),
    "openai_key": re.compile(rb"(?i)\bsk-(?:proj|live)-[A-Za-z0-9_-]{20,}"),
    "aws_access_key": re.compile(rb"\bAKIA[0-9A-Z]{16}\b"),
    "private_key_pem": re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "url_userinfo": re.compile(rb"(?i)\bhttps?://[^/\s:@]+:[^@\s/]+@"),
    "bearer_credential": re.compile(rb"(?i)\bBearer\s+[A-Za-z0-9_~+./=-]{20,}"),
}
CACHE_MARKERS = ("/__pycache__/", ".pyc", ".pyo", "/target/", ".DS_Store")


def git_paths(arguments: list[str]) -> list[str]:
    result = subprocess.run(["git", *arguments], cwd=ROOT, check=True, capture_output=True)
    return [path.decode("utf-8", "surrogateescape") for path in result.stdout.split(b"\0") if path]


def contains_git_directory(path: Path) -> bool:
    return path.is_dir() and (path / ".git").exists()


all_paths = sorted(
    set(git_paths(["ls-files", "--cached", "-z"]))
    | set(git_paths(["ls-files", "--others", "--exclude-standard", "-z"]))
)
matches: list[dict[str, str]] = []
excluded: dict[str, list[str]] = {
    "claude_local_state": [],
    "generated_cache": [],
    "embedded_git_repository": [],
    "audit_output": [],
    "control_manifest": [],
    "symlink": [],
    "non_regular": [],
}
scanned_files = 0
scanned_bytes = 0
zip_members = 0
archive_errors: list[str] = []
export_bytes = 0
export_regular_files = 0


def inspect(data: bytes, *, path: str, member: str | None = None) -> None:
    for label, pattern in PATTERNS.items():
        if pattern.search(data):
            row = {"path": path, "pattern": label}
            if member is not None:
                row["member"] = member
            matches.append(row)


for relative in all_paths:
    normalized = "/" + relative.replace("\\", "/")
    path = ROOT / relative.rstrip("/")
    if relative == AUDIT_OUTPUT:
        excluded["audit_output"].append(relative)
        continue
    if relative in CONTROL_MANIFESTS:
        excluded["control_manifest"].append(relative)
        continue
    if normalized.startswith("/.claude/") or normalized == "/.claude":
        excluded["claude_local_state"].append(relative)
        continue
    if any(marker in normalized for marker in CACHE_MARKERS):
        excluded["generated_cache"].append(relative)
        continue
    if contains_git_directory(path):
        excluded["embedded_git_repository"].append(relative)
        continue
    if path.is_symlink():
        excluded["symlink"].append(relative)
        continue
    if not path.is_file():
        excluded["non_regular"].append(relative)
        continue
    data = path.read_bytes()
    export_bytes += len(data)
    export_regular_files += 1
    scanned_files += 1
    scanned_bytes += len(data)
    inspect(data, path=relative)
    if path.suffix.lower() == ".zip":
        try:
            with zipfile.ZipFile(path) as archive:
                for info in archive.infolist():
                    if info.is_dir():
                        continue
                    zip_members += 1
                    inspect(archive.read(info), path=relative, member=info.filename)
        except (OSError, zipfile.BadZipFile, RuntimeError) as error:
            archive_errors.append(f"{relative}: {type(error).__name__}")

print(
    json.dumps(
        {
            "all_indexed_or_untracked_nonignored_paths": len(all_paths),
            "export_regular_files": export_regular_files,
            "export_bytes": export_bytes,
            "files_scanned": scanned_files,
            "file_bytes_scanned": scanned_bytes,
            "zip_members_scanned": zip_members,
            "high_confidence_match_count": len(matches),
            "high_confidence_matches": matches,
            "excluded": {key: {"count": len(value), "paths": value} for key, value in excluded.items()},
            "archive_errors": archive_errors,
            "patterns": sorted(PATTERNS),
            "scope_note": (
                "Includes all tracked and untracked non-ignored paths except .claude local state, "
                "generated caches, embedded Git repositories, integrity manifests, the audit output, "
                "symlinks, and non-regular paths. "
                "Ignored paths (including .runtime), environment, process memory, and email are not read. "
                "Pattern scan is targeted and is not a guarantee that every secret format is absent."
            ),
        },
        sort_keys=True,
        indent=2,
    )
)
