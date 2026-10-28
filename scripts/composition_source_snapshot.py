"""Capture bounded, hash-addressed source inputs for local composed runs."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import stat
import zipfile


MAX_SOURCE_SNAPSHOT_BYTES = 64 * 1024 * 1024
MAX_SOURCE_SNAPSHOT_FILES = 10_000
MAX_SOURCE_SNAPSHOT_ARCHIVE_BYTES = 128 * 1024 * 1024
SOURCE_SUFFIXES = {".json", ".py", ".rs", ".sh", ".toml"}
EXCLUDED_DIRS = {".git", ".runtime", ".claude", "__pycache__", "evidence", "node_modules", "target"}
SOURCE_DIRS = ("scripts", "src", "tests", "requirements", "policy", "policies")
ROOT_SOURCE_FILES = ("Cargo.toml", "Cargo.lock", "build.rs", "rust-toolchain.toml", "rustfmt.toml")
CONFIG_SOURCE_FILES = (".cargo/config.toml",)


class SourceSnapshotError(RuntimeError):
    """A source snapshot could not be captured or verified safely."""


def discover_source_paths(repo_root: Path) -> list[str]:
    root = repo_root.resolve(strict=True)
    if not root.is_dir():
        raise SourceSnapshotError("repository root must be a directory")

    paths: set[str] = set()
    for relative in (*ROOT_SOURCE_FILES, *CONFIG_SOURCE_FILES):
        path = root / relative
        if path.exists() or path.is_symlink():
            if _has_symlink_component(root, relative):
                raise SourceSnapshotError(f"symbolic link in source path: {relative}")
            paths.add(relative)

    for directory in SOURCE_DIRS:
        source_root = root / directory
        if not source_root.exists() and not source_root.is_symlink():
            continue
        if source_root.is_symlink() or not source_root.is_dir():
            raise SourceSnapshotError(f"source directory is not a regular directory: {directory}")
        for current, names, filenames in os.walk(source_root, topdown=True, followlinks=False):
            current_path = Path(current)
            for name in tuple(names):
                child = current_path / name
                if name in EXCLUDED_DIRS:
                    names.remove(name)
                elif child.is_symlink():
                    raise SourceSnapshotError(f"symbolic link in source tree: {child.relative_to(root)}")
            for filename in filenames:
                path = current_path / filename
                if path.suffix not in SOURCE_SUFFIXES:
                    continue
                if path.is_symlink():
                    raise SourceSnapshotError(f"symbolic link in source tree: {path.relative_to(root)}")
                paths.add(path.relative_to(root).as_posix())

    if not paths:
        raise SourceSnapshotError("no source inputs were found")
    if len(paths) > MAX_SOURCE_SNAPSHOT_FILES:
        raise SourceSnapshotError("source file count exceeds the snapshot limit")
    return sorted(paths)


def capture_sources(repo_root: Path, destination: Path, paths: list[str]) -> dict[str, object]:
    root = repo_root.resolve(strict=True)
    if destination.exists() or destination.is_symlink():
        raise SourceSnapshotError("source snapshot destination already exists")
    if (
        not destination.parent.is_dir()
        or destination.parent.is_symlink()
        or destination.parent.resolve(strict=True) != destination.parent.absolute()
    ):
        raise SourceSnapshotError("source snapshot parent must be a regular directory")

    source_bytes: dict[str, bytes] = {}
    source_hashes: dict[str, str] = {}
    total_bytes = 0
    for relative in sorted(set(paths)):
        candidate = PurePosixPath(relative)
        if candidate.is_absolute() or not candidate.parts or any(part in {"", ".", ".."} for part in candidate.parts):
            raise SourceSnapshotError(f"invalid source path: {relative!r}")
        if _has_symlink_component(root, relative):
            raise SourceSnapshotError(f"symbolic link in source path: {relative}")
        path = root.joinpath(*candidate.parts)
        try:
            path.resolve(strict=True).relative_to(root)
        except (OSError, ValueError):
            raise SourceSnapshotError(f"source path escapes the repository: {relative}") from None
        if path.is_symlink():
            raise SourceSnapshotError(f"symbolic link in source tree: {relative}")
        try:
            metadata = path.stat(follow_symlinks=False)
        except OSError:
            raise SourceSnapshotError(f"source file is missing: {relative}") from None
        if not stat.S_ISREG(metadata.st_mode):
            raise SourceSnapshotError(f"source is not a regular file: {relative}")
        if metadata.st_size > MAX_SOURCE_SNAPSHOT_BYTES - total_bytes:
            raise SourceSnapshotError("source bytes exceed the snapshot limit")
        data = path.read_bytes()
        if len(data) != metadata.st_size:
            raise SourceSnapshotError(f"source changed while being captured: {relative}")
        total_bytes += len(data)
        source_bytes[relative] = data
        source_hashes[relative] = hashlib.sha256(data).hexdigest()

    if not source_bytes:
        raise SourceSnapshotError("source snapshot must contain at least one file")

    manifest = {
        "schema_version": 1,
        "file_count": len(source_bytes),
        "total_bytes": total_bytes,
        "files": [
            {"path": relative, "sha256": source_hashes[relative], "size_bytes": len(source_bytes[relative])}
            for relative in sorted(source_bytes)
        ],
    }
    manifest_bytes = (json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n").encode()
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(destination, flags, 0o600)
        with os.fdopen(fd, "wb") as output:
            with zipfile.ZipFile(output, mode="w", compression=zipfile.ZIP_STORED) as archive:
                _write_zip_member(archive, "source-manifest.json", manifest_bytes)
                for relative, data in sorted(source_bytes.items()):
                    _write_zip_member(archive, f"sources/{relative}", data)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError:
        raise SourceSnapshotError("source snapshot destination already exists") from None
    except Exception:
        try:
            destination.unlink()
        except FileNotFoundError:
            pass
        raise

    archive_sha256 = sha256_file(destination)
    return {
        "schema_version": 1,
        "archive_name": destination.name,
        "archive_sha256": archive_sha256,
        "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
        "source_sha256": source_hashes,
        "file_count": len(source_bytes),
        "total_bytes": total_bytes,
    }


def verify_sources_unchanged(repo_root: Path, expected_hashes: dict[str, str]) -> None:
    current_paths = discover_source_paths(repo_root)
    if set(current_paths) != set(expected_hashes):
        raise SourceSnapshotError("source file set changed after snapshot")
    root = repo_root.resolve(strict=True)
    for relative, expected in expected_hashes.items():
        path = root.joinpath(*PurePosixPath(relative).parts)
        try:
            actual = sha256_file(path)
        except (OSError, SourceSnapshotError):
            raise SourceSnapshotError(f"source changed after snapshot: {relative}") from None
        if actual != expected:
            raise SourceSnapshotError(f"source changed after snapshot: {relative}")


def copy_snapshot(source: Path, destination: Path, expected_sha256: str) -> str:
    if source.is_symlink() or not source.is_file() or destination.is_symlink():
        raise SourceSnapshotError("snapshot copy source or destination is unsafe")
    if (
        not destination.parent.is_dir()
        or destination.parent.is_symlink()
        or destination.parent.resolve(strict=True) != destination.parent.absolute()
    ):
        raise SourceSnapshotError("snapshot evidence parent must be a regular directory")
    if destination.exists():
        raise SourceSnapshotError("snapshot evidence destination already exists")
    if source.stat().st_size > MAX_SOURCE_SNAPSHOT_ARCHIVE_BYTES:
        raise SourceSnapshotError("source snapshot archive exceeds the copy limit")
    with source.open("rb") as source_file:
        data = source_file.read(MAX_SOURCE_SNAPSHOT_ARCHIVE_BYTES + 1)
    if len(data) > MAX_SOURCE_SNAPSHOT_ARCHIVE_BYTES:
        raise SourceSnapshotError("source snapshot archive exceeds the copy limit")
    digest = hashlib.sha256(data).hexdigest()
    if digest != expected_sha256:
        raise SourceSnapshotError("source snapshot archive hash changed before evidence copy")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(destination, flags, 0o600)
    except FileExistsError:
        raise SourceSnapshotError("snapshot evidence destination already exists") from None
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
    except Exception:
        try:
            destination.unlink()
        except FileNotFoundError:
            pass
        raise
    if sha256_file(destination) != expected_sha256:
        raise SourceSnapshotError("source snapshot evidence copy failed integrity check")
    return digest


def sha256_file(path: Path) -> str:
    if path.is_symlink() or not path.is_file():
        raise SourceSnapshotError(f"not a regular file: {path}")
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _has_symlink_component(root: Path, relative: str) -> bool:
    current = root
    for part in PurePosixPath(relative).parts:
        current = current / part
        if current.is_symlink():
            return True
    return False


def _write_zip_member(archive: zipfile.ZipFile, name: str, data: bytes) -> None:
    info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
    info.create_system = 3
    info.external_attr = 0o100600 << 16
    archive.writestr(info, data)
