#!/usr/bin/env python3
"""Unit checks for the loopback Forgejo native webhook replay runner."""

from __future__ import annotations

import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import run_forgejo_live_webhook_native_replay_e2e as driver


class NativeReplayBindingTests(unittest.TestCase):
    def setUp(self) -> None:
        payload_semantic_sha256 = driver.canonical_json_sha256({"action": "opened"})
        self.original = {
            "hook_id": 1,
            "uuid": "delivery-original",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": False,
            "response_status": 401,
            "payload_sha256": "a" * 64,
            "payload_semantic_sha256": payload_semantic_sha256,
        }
        self.replay = {
            "hook_id": 1,
            "uuid": "delivery-replay",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": True,
            "response_status": 202,
            "payload_sha256": "a" * 64,
            "payload_semantic_sha256": payload_semantic_sha256,
        }
        self.journal_record = {
            "sequence": 1,
            "kind": "forgejo.webhook.v1.issues",
            "idempotency_key": "forgejo-delivery-v1:delivery-replay",
            "payload": {
                "schema_version": 1,
                "provider": "forgejo",
                "event": "issues",
                "delivery_id": "delivery-replay",
                "payload": {"action": "opened"},
            },
        }

    def test_binds_failed_original_to_successful_new_uuid_and_same_payload(self) -> None:
        result = driver.validate_native_replay_task(
            self.original, self.replay, self.journal_record
        )
        self.assertEqual(result["original_uuid"], "delivery-original")
        self.assertEqual(result["replay_uuid"], "delivery-replay")
        self.assertIs(result["new_delivery_id"], True)
        self.assertIs(result["payload_unchanged"], True)
        self.assertEqual(result["http_status"], 202)
        self.assertEqual(result["sequence"], 1)

    def test_rejects_same_uuid_or_changed_payload(self) -> None:
        for change in ("uuid", "payload_sha256"):
            with self.subTest(change=change):
                replay = copy.deepcopy(self.replay)
                replay[change] = (
                    "b" * 64 if change == "payload_sha256" else self.original[change]
                )
                with self.assertRaises(ValueError):
                    driver.validate_native_replay_task(
                        self.original, replay, self.journal_record
                    )

    def test_rejects_nonfailed_original_or_unverified_replay(self) -> None:
        original = copy.deepcopy(self.original)
        original["is_succeed"] = True
        with self.assertRaises(ValueError):
            driver.validate_native_replay_task(
                original, self.replay, self.journal_record
            )

        replay = copy.deepcopy(self.replay)
        replay["response_status"] = 500
        with self.assertRaises(ValueError):
            driver.validate_native_replay_task(
                self.original, replay, self.journal_record
            )

    def test_rejects_journal_record_bound_to_another_uuid(self) -> None:
        record = copy.deepcopy(self.journal_record)
        record["payload"]["delivery_id"] = "delivery-other"
        with self.assertRaises(ValueError):
            driver.validate_native_replay_task(self.original, self.replay, record)

    def test_rejects_journal_payload_that_differs_from_forgejo_task(self) -> None:
        record = copy.deepcopy(self.journal_record)
        record["payload"]["payload"]["action"] = "closed"
        with self.assertRaises(ValueError):
            driver.validate_native_replay_task(self.original, self.replay, record)

    def test_binds_repeated_manual_replays_to_distinct_ordered_deliveries(self) -> None:
        second_replay = copy.deepcopy(self.replay)
        second_replay["uuid"] = "delivery-replay-second"
        second_record = copy.deepcopy(self.journal_record)
        second_record["sequence"] = 2
        second_record["idempotency_key"] = "forgejo-delivery-v1:delivery-replay-second"
        second_record["payload"]["delivery_id"] = "delivery-replay-second"

        result = driver.validate_native_replay_chain(
            self.original,
            [self.replay, second_replay],
            [self.journal_record, second_record],
        )
        self.assertEqual(result["replay_count"], 2)
        self.assertEqual(
            result["replay_uuids"], ["delivery-replay", "delivery-replay-second"]
        )
        self.assertEqual([item["sequence"] for item in result["deliveries"]], [1, 2])

        duplicate_uuid = copy.deepcopy(second_replay)
        duplicate_uuid["uuid"] = self.replay["uuid"]
        duplicate_record = copy.deepcopy(second_record)
        duplicate_record["idempotency_key"] = self.journal_record["idempotency_key"]
        duplicate_record["payload"]["delivery_id"] = self.journal_record["payload"]["delivery_id"]
        with self.assertRaises(ValueError):
            driver.validate_native_replay_chain(
                self.original,
                [self.replay, duplicate_uuid],
                [self.journal_record, duplicate_record],
            )

        wrong_order = copy.deepcopy(second_record)
        wrong_order["sequence"] = 3
        with self.assertRaises(ValueError):
            driver.validate_native_replay_chain(
                self.original,
                [self.replay, second_replay],
                [self.journal_record, wrong_order],
            )

    def test_binds_inactive_hook_replay_to_skipped_task_and_unchanged_journal(self) -> None:
        inactive_task = copy.deepcopy(self.replay)
        inactive_task.update(
            uuid="delivery-replay-inactive",
            is_delivered=True,
            is_succeed=False,
            response_status=0,
        )
        proxy_before = [{"delivery_id": "delivery-replay", "http_status": 202}]
        proxy_after = copy.deepcopy(proxy_before)
        before = [self.journal_record]
        after = copy.deepcopy(before)
        result = driver.validate_inactive_hook_replay(
            {"id": 1, "active": False},
            303,
            self.original,
            inactive_task,
            proxy_before,
            proxy_after,
            before,
            after,
        )
        self.assertIs(result["task_created"], True)
        self.assertIs(result["hook_active"], False)
        self.assertIs(result["task_marked_delivered"], True)
        self.assertIs(result["task_succeeded"], False)
        self.assertIs(result["receiver_journal_unchanged"], True)

        invalid_cases = (
            ({"id": 1, "active": True}, 303, inactive_task, proxy_before, proxy_after, before, after),
            ({"id": 1, "active": False}, 403, inactive_task, proxy_before, proxy_after, before, after),
            (
                {"id": 1, "active": False},
                303,
                {**inactive_task, "response_status": 202, "is_succeed": True},
                proxy_before,
                proxy_after,
                before,
                after,
            ),
            (
                {"id": 1, "active": False},
                303,
                inactive_task,
                proxy_before,
                [*proxy_after, {"delivery_id": "unexpected", "http_status": 202}],
                before,
                after,
            ),
            (
                {"id": 1, "active": False},
                303,
                inactive_task,
                proxy_before,
                proxy_after,
                before,
                [*after, self.journal_record],
            ),
        )
        for hook, status, task, proxy_old, proxy_new, journal_before, journal_after in invalid_cases:
            with self.subTest(hook=hook, status=status, task=task, proxy_new=proxy_new):
                with self.assertRaises(ValueError):
                    driver.validate_inactive_hook_replay(
                        hook,
                        status,
                        self.original,
                        task,
                        proxy_old,
                        proxy_new,
                        journal_before,
                        journal_after,
                    )

    def test_binds_every_proxy_request_body_to_its_forgejo_task(self) -> None:
        second_replay = copy.deepcopy(self.replay)
        second_replay["uuid"] = "delivery-replay-second"
        tasks = [self.original, self.replay, second_replay]
        records = [
            {
                "method": "POST",
                "path": "/forgejo/webhook",
                "delivery_id": task["uuid"],
                "event_type": "issues",
                "request_body_length": 20,
                "request_body_sha256": task["payload_sha256"],
                "forwarded_status": task["response_status"],
            }
            for task in tasks
        ]
        result = driver.validate_proxy_delivery_attempts(records, tasks)
        self.assertEqual(result["delivery_count"], 3)
        self.assertEqual(result["delivery_ids"], [task["uuid"] for task in tasks])
        self.assertIs(result["raw_payload_hashes_match"], True)

        changed_body = copy.deepcopy(records)
        changed_body[1]["request_body_sha256"] = "b" * 64
        with self.assertRaises(ValueError):
            driver.validate_proxy_delivery_attempts(changed_body, tasks)

        duplicate_request = copy.deepcopy(records)
        duplicate_request.append(copy.deepcopy(records[-1]))
        with self.assertRaises(ValueError):
            driver.validate_proxy_delivery_attempts(duplicate_request, tasks)

    def test_accepts_only_loopback_http_origins_with_a_valid_port(self) -> None:
        validate = driver.validate_loopback_origin

        self.assertEqual(validate("http://127.0.0.1:3132"), "http://127.0.0.1:3132")
        for origin in (
            "https://127.0.0.1:3132",
            "http://localhost:3132",
            "http://127.0.0.2:3132",
            "http://127.0.0.1:0",
            "http://127.0.0.1:65536",
            "http://127.0.0.1:3132/path",
            "http://user@127.0.0.1:3132",
        ):
            with self.subTest(origin=origin), self.assertRaises(ValueError):
                validate(origin)


if __name__ == "__main__":
    unittest.main()
