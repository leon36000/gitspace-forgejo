#!/usr/bin/env python3
"""Scan changed Git paths and archive members for high-confidence credential formats."""

from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
PATTERNS = {
    "gitlab_pat": re.compile(rb"(?i)\bglpat-[A-Za-z0-9_-]{20,}"),
    "gitlab_runner_token": re.compile(rb"(?i)\bglrt-[A-Za-z0-9_-]{20,}"),
    "github_token": re.compile(rb"(?i)\bgh[pousr]_[A-Za-z0-9]{20,}"),
    "openai_key": re.compile(rb"(?i)\bsk-(?:proj|live)-[A-Za-z0-9_-]{20,}"),
    "aws_access_key": re.compile(rb"\bAKIA[0-9A-Z]{16}\b"),
    "private_key_pem": re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "url_userinfo": re.compile(rb"(?i)\bhttps?://[^/\s:@]+:[^@\s/]+@"),
}
CACHE_MARKERS = ("/__pycache__/", ".pyc", ".pyo", "/target/", ".DS_Store")


def git_paths(args: list[str]) -> list[str]:
    result = subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True)
    return [p.decode("utf-8", "surrogateescape") for p in result.stdout.split(b"\0") if p]


paths = sorted(
    set(git_paths(["diff", "--name-only", "-z"]))
    | set(git_paths(["diff", "--cached", "--name-only", "-z"]))
    | set(git_paths(["ls-files", "--others", "--exclude-standard", "-z"]))
)
matches: list[dict[str, str]] = []
scanned_files = 0
scanned_bytes = 0
archive_members = 0
archive_errors: list[str] = []
symlinks: list[str] = []
cache_paths: list[str] = []


def inspect(data: bytes, *, path: str, member: str | None = None) -> None:
    for label, pattern in PATTERNS.items():
        if pattern.search(data):
            row = {"path": path, "pattern": label}
            if member is not None:
                row["member"] = member
            matches.append(row)


for rel in paths:
    normalized = "/" + rel.replace("\\", "/")
    if any(marker in normalized for marker in CACHE_MARKERS):
        cache_paths.append(rel)
    path = ROOT / rel
    if path.is_symlink():
        symlinks.append(rel)
        continue
    if not path.is_file():
        continue
    data = path.read_bytes()
    scanned_files += 1
    scanned_bytes += len(data)
    inspect(data, path=rel)
    if path.suffix.lower() == ".zip":
        try:
            with zipfile.ZipFile(path) as archive:
                for info in archive.infolist():
                    if info.is_dir():
                        continue
                    archive_members += 1
                    inspect(archive.read(info), path=rel, member=info.filename)
        except (OSError, zipfile.BadZipFile, RuntimeError) as error:
            archive_errors.append(f"{rel}: {type(error).__name__}")

print(
    json.dumps(
        {
            "changed_and_untracked_paths": len(paths),
            "regular_files_scanned": scanned_files,
            "file_bytes_scanned": scanned_bytes,
            "zip_members_scanned": archive_members,
            "high_confidence_match_count": len(matches),
            "high_confidence_matches": matches,
            "generated_cache_path_count": len(cache_paths),
            "generated_cache_paths": cache_paths,
            "symlink_path_count": len(symlinks),
            "symlink_paths": symlinks,
            "archive_errors": archive_errors,
            "patterns": sorted(PATTERNS),
            "scope_note": "Changed, staged, and untracked non-ignored paths only; ignored files, environment, memory, and email are outside scope.",
        },
        sort_keys=True,
        indent=2,
    )
)
