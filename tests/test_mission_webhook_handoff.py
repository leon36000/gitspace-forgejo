from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import mission_webhook_handoff as handoff  # noqa: E402


class WebhookAdmissionHandoffTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="webhook-admission-handoff-")
        self.addCleanup(self.temporary.cleanup)
        self.runtime = Path(self.temporary.name) / "project" / ".runtime"
        self.runtime.mkdir(mode=0o700, parents=True)
        self.scope = {
            "origin": "http://127.0.0.1:3132",
            "owner": "forge-admin",
            "repository": "mission-012-clamp",
            "branch": "main",
        }
        self.host_admission = {
            "schema_version": 1,
            "scope": self.scope,
            "issue_number": 1,
            "requirement_sha256": "a" * 64,
            "allocation": {
                "schema_version": 1,
                "allocation_id": "mission012-handoff-001",
                "mission_id": "mission-v1-" + "b" * 64,
            },
        }
        self.source = {
            "schema_version": 1,
            "delivery_id": "delivery-001",
            "journal_sequence": 1,
            "journal_event_id": "evt-1",
            "journal_kind": "forgejo.webhook.v1.issues",
            "journal_event_sha256": "c" * 64,
            "allocation_id": "mission012-handoff-001",
            "mission_id": "mission-v1-" + "b" * 64,
            "requirement_sha256": "a" * 64,
            "issue_number": 1,
            "scope": self.scope,
        }
        self.admission_bytes = json.dumps(
            self.host_admission, sort_keys=True, separators=(",", ":")
        ).encode() + b"\n"
        self.source_bytes = json.dumps(
            self.source, sort_keys=True, separators=(",", ":")
        ).encode() + b"\n"
        self.delivery_source = self.runtime / (
            "mission-host-admission-source-"
            + hashlib.sha256(b"delivery-001").hexdigest()
            + ".json"
        )
        self._private_write(self.runtime / "host-admission.json", self.admission_bytes)
        self._private_write(self.delivery_source, self.source_bytes)
        self._private_write(
            self.runtime / "forgejo-webhook-inbox.jsonl",
            b'{"delivery_id":"delivery-001"}\n',
        )
        self.result = {
            "mission": {
                "allocation_id": "mission012-handoff-001",
                "requirement_sha256": "a" * 64,
                "admission_file_sha256": hashlib.sha256(self.admission_bytes).hexdigest(),
                "admission": {
                    "admission_mode": "webhook_delivery",
                    "host_admission_file": str(self.runtime / "host-admission.json"),
                    "host_admission_source_file": str(self.delivery_source),
                    "host_admission_source": self.source,
                    "host_admission": self.host_admission,
                },
            },
            "webhook": {
                "event": {
                    "delivery_id": "delivery-001",
                    "sequence": 1,
                    "event": "issues",
                    "issue_number": 1,
                }
            },
        }

    def _private_write(self, path: Path, data: bytes) -> None:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)

    def test_admission_handoff_is_bound_to_the_exact_private_delivery_receipt(self):
        result = handoff.validate_stage_admission_handoff(self.result, self.runtime)
        self.assertEqual(result["delivery_id"], "delivery-001")
        self.assertEqual(result["admission_source_file"], str(self.delivery_source))
        self.assertEqual(result["webhook_inbox_file"], str(self.runtime / "forgejo-webhook-inbox.jsonl"))
        self.assertEqual(result["admission_source_sha256"], hashlib.sha256(self.source_bytes).hexdigest())

        agent_result = handoff.validate_agent_handoff_files(
            self.runtime,
            self.runtime / "host-admission.json",
            self.runtime / "forgejo-webhook-inbox.jsonl",
            self.delivery_source,
            self.host_admission,
        )
        self.assertEqual(agent_result["delivery_id"], result["delivery_id"])
        self.assertEqual(agent_result["admission_source_sha256"], result["admission_source_sha256"])
        self.assertEqual(agent_result["webhook_inbox_sha256"], result["webhook_inbox_sha256"])

    def test_admission_handoff_rejects_a_source_record_for_another_delivery(self):
        altered = dict(self.source, delivery_id="delivery-002")
        self.result["mission"]["admission"]["host_admission_source"] = altered
        with self.assertRaisesRegex(RuntimeError, "delivery identity"):
            handoff.validate_stage_admission_handoff(self.result, self.runtime)

    def test_admission_handoff_rejects_source_path_outside_the_runtime(self):
        self.result["mission"]["admission"]["host_admission_source_file"] = "/tmp/untrusted.json"
        with self.assertRaisesRegex(RuntimeError, "path does not match"):
            handoff.validate_stage_admission_handoff(self.result, self.runtime)

    def test_agent_handoff_rejects_symlinked_source_receipt(self):
        self.delivery_source.unlink()
        external = Path(self.temporary.name) / "external.json"
        external.write_bytes(self.source_bytes)
        external.chmod(0o600)
        self.delivery_source.symlink_to(external)
        with self.assertRaisesRegex(RuntimeError, "unavailable or unsafe"):
            handoff.validate_agent_handoff_files(
                self.runtime,
                self.runtime / "host-admission.json",
                self.runtime / "forgejo-webhook-inbox.jsonl",
                self.delivery_source,
                self.host_admission,
            )


if __name__ == "__main__":
    unittest.main()
