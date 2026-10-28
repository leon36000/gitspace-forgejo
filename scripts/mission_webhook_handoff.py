"""Validate the private webhook receipt passed from admission to the MCP host."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import stat


MAX_SOURCE_RECEIPT_BYTES = 64 * 1024
MAX_WEBHOOK_INBOX_BYTES = 16 * 1024 * 1024


def read_private_regular_file(path: Path, max_bytes: int) -> bytes:
    if not path.is_absolute() or max_bytes <= 0:
        raise RuntimeError("webhook admission handoff path or size limit is invalid")
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK,
        )
    except OSError:
        raise RuntimeError("webhook admission handoff file is unavailable or unsafe") from None
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        if (
            not stat.S_ISREG(before.st_mode)
            or before.st_uid != os.geteuid()
            or before.st_nlink != 1
            or stat.S_IMODE(before.st_mode) != 0o600
            or before.st_size > max_bytes
        ):
            raise RuntimeError("webhook admission handoff file is not a bounded private regular file")
        payload = stream.read(max_bytes + 1)
        after = os.fstat(stream.fileno())
        if (
            len(payload) > max_bytes
            or len(payload) != before.st_size
            or (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
            != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
        ):
            raise RuntimeError("webhook admission handoff file changed while being read")
        return payload


def _private_runtime_dir(path: Path) -> None:
    try:
        metadata = path.lstat()
    except OSError:
        raise RuntimeError("webhook admission runtime directory is unavailable") from None
    if (
        not path.is_absolute()
        or stat.S_ISLNK(metadata.st_mode)
        or not stat.S_ISDIR(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o700
    ):
        raise RuntimeError("webhook admission runtime directory is not private")


def _sha256(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def _parse_json(payload: bytes) -> dict[str, object]:
    try:
        value = json.loads(payload)
    except (json.JSONDecodeError, UnicodeDecodeError):
        raise RuntimeError("webhook admission source receipt is not valid JSON") from None
    if not isinstance(value, dict):
        raise RuntimeError("webhook admission source receipt is not a JSON object")
    return value


def _validate_source_record(source: dict[str, object], host_admission: dict[str, object]) -> None:
    allocation = host_admission.get("allocation")
    if not isinstance(allocation, dict):
        raise RuntimeError("webhook source receipt does not bind a host allocation")
    delivery_id = source.get("delivery_id")
    sequence = source.get("journal_sequence")
    if (
        source.get("schema_version") != 1
        or not isinstance(delivery_id, str)
        or re.fullmatch(r"[A-Za-z0-9_-]{1,128}", delivery_id) is None
        or not isinstance(sequence, int)
        or isinstance(sequence, bool)
        or sequence <= 0
        or not isinstance(source.get("journal_event_id"), str)
        or not source["journal_event_id"]
        or not isinstance(source.get("journal_event_sha256"), str)
        or re.fullmatch(r"[0-9a-f]{64}", source["journal_event_sha256"]) is None
        or not isinstance(source.get("journal_kind"), str)
        or source.get("allocation_id") != allocation.get("allocation_id")
        or source.get("mission_id") != allocation.get("mission_id")
        or source.get("requirement_sha256") != host_admission.get("requirement_sha256")
        or source.get("issue_number") != host_admission.get("issue_number")
        or source.get("scope") != host_admission.get("scope")
    ):
        raise RuntimeError("webhook source receipt is not bound to the private host admission")


def _runtime_paths(runtime_dir: Path) -> tuple[Path, Path, Path]:
    _private_runtime_dir(runtime_dir)
    return (
        runtime_dir / "host-admission.json",
        runtime_dir / "forgejo-webhook-inbox.jsonl",
        runtime_dir,
    )


def validate_stage_admission_handoff(
    admission_result: dict[str, object], runtime_dir: Path
) -> dict[str, str]:
    admission_path, inbox_path, _ = _runtime_paths(runtime_dir)
    mission = admission_result.get("mission")
    webhook = admission_result.get("webhook")
    if not isinstance(mission, dict) or not isinstance(webhook, dict):
        raise RuntimeError("webhook admission result omitted its mission or event")
    cli_admission = mission.get("admission")
    if not isinstance(cli_admission, dict) or cli_admission.get("admission_mode") != "webhook_delivery":
        raise RuntimeError("MCP host admission is not bound to a webhook delivery")
    if cli_admission.get("host_admission_file") != str(admission_path):
        raise RuntimeError("webhook admission host file is outside its private runtime directory")
    source_path_value = cli_admission.get("host_admission_source_file")
    source_record_value = cli_admission.get("host_admission_source")
    event = webhook.get("event")
    host_admission_value = cli_admission.get("host_admission")
    if (
        not isinstance(source_path_value, str)
        or not isinstance(source_record_value, dict)
        or not isinstance(event, dict)
        or not isinstance(host_admission_value, dict)
    ):
        raise RuntimeError("webhook admission result omitted its per-delivery source receipt")
    source_record = dict(source_record_value)
    _validate_source_record(source_record, host_admission_value)
    delivery_id = source_record["delivery_id"]
    expected_source = runtime_dir / (
        "mission-host-admission-source-" + _sha256(delivery_id.encode()) + ".json"
    )
    if Path(source_path_value) != expected_source:
        raise RuntimeError("webhook source receipt path does not match its delivery identity")
    if (
        event.get("delivery_id") != delivery_id
        or event.get("sequence") != source_record.get("journal_sequence")
        or event.get("issue_number") != source_record.get("issue_number")
        or event.get("event") != source_record.get("journal_kind", "").removeprefix("forgejo.webhook.v1.")
        or mission.get("allocation_id") != source_record.get("allocation_id")
        or mission.get("requirement_sha256") != source_record.get("requirement_sha256")
    ):
        raise RuntimeError("webhook source receipt does not match the admitted event")

    admission_bytes = read_private_regular_file(admission_path, MAX_SOURCE_RECEIPT_BYTES)
    if _sha256(admission_bytes) != mission.get("admission_file_sha256"):
        raise RuntimeError("private host admission file differs from its reported digest")
    if _parse_json(admission_bytes) != host_admission_value:
        raise RuntimeError("private host admission file differs from the admitted record")
    source_bytes = read_private_regular_file(expected_source, MAX_SOURCE_RECEIPT_BYTES)
    if _parse_json(source_bytes) != source_record:
        raise RuntimeError("private webhook source receipt differs from the admission result")
    inbox_bytes = read_private_regular_file(inbox_path, MAX_WEBHOOK_INBOX_BYTES)
    return {
        "webhook_inbox_file": str(inbox_path),
        "admission_source_file": str(expected_source),
        "delivery_id": delivery_id,
        "admission_source_sha256": _sha256(source_bytes),
        "webhook_inbox_sha256": _sha256(inbox_bytes),
    }


def validate_agent_handoff_files(
    runtime_dir: Path,
    admission_file: Path,
    webhook_inbox_file: Path,
    admission_source_file: Path,
    host_admission: dict[str, object],
) -> dict[str, str]:
    expected_admission, expected_inbox, _ = _runtime_paths(runtime_dir)
    if (
        admission_file != expected_admission
        or webhook_inbox_file != expected_inbox
        or not admission_source_file.is_absolute()
        or admission_source_file.parent != runtime_dir
    ):
        raise RuntimeError("MCP host webhook handoff paths are outside the private runtime directory")
    admission_bytes = read_private_regular_file(expected_admission, MAX_SOURCE_RECEIPT_BYTES)
    if _parse_json(admission_bytes) != host_admission:
        raise RuntimeError("MCP host admission file changed before webhook handoff")
    source_bytes = read_private_regular_file(admission_source_file, MAX_SOURCE_RECEIPT_BYTES)
    source = _parse_json(source_bytes)
    _validate_source_record(source, host_admission)
    expected_source = runtime_dir / (
        "mission-host-admission-source-" + _sha256(source["delivery_id"].encode()) + ".json"
    )
    if admission_source_file != expected_source:
        raise RuntimeError("MCP host source receipt path does not match its delivery identity")
    inbox_bytes = read_private_regular_file(expected_inbox, MAX_WEBHOOK_INBOX_BYTES)
    return {
        "delivery_id": source["delivery_id"],
        "admission_source_sha256": _sha256(source_bytes),
        "webhook_inbox_sha256": _sha256(inbox_bytes),
    }
