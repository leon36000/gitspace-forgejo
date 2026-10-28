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

    def test_accepts_server_busy_as_a_failed_delivery_for_manual_replay(self) -> None:
        original = copy.deepcopy(self.original)
        original["response_status"] = 503
        result = driver.validate_native_replay_task(
            original, self.replay, self.journal_record
        )
        self.assertEqual(result["http_status"], 202)
        self.assertIs(result["new_delivery_id"], True)

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


class NativeReplayReconciliationTests(unittest.TestCase):
    def setUp(self) -> None:
        validator = getattr(driver, "validate_native_replay_reconciliation_batch", None)
        self.assertTrue(callable(validator), "native replay reconciliation validator is missing")
        self.validate_batch = validator
        self.issue = {
            "number": 1,
            "state": "open",
            "updated_at": "2026-10-25T12:00:00-04:00",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/1",
        }
        self.delivery_ids = ["native-replay-001", "native-replay-002"]
        self.batch = {
            "schema_version": 23,
            "api_version": "forgejo-rest-v1",
            "verdict": "current_state_read",
            "upstream_read_attempts": 1,
            "results": [
                {
                    "delivery_id": delivery_id,
                    "verdict": "current_state_read",
                    "resource": {
                        "kind": "issue",
                        "owner": "forge-admin",
                        "repository": "agent-forge-core",
                        "number": 1,
                    },
                    "canonical_state": {
                        "state": "open",
                        "updated_at": self.issue["updated_at"],
                        "html_url": self.issue["html_url"],
                    },
                }
                for delivery_id in self.delivery_ids
            ],
        }

    def _validate(self, batch=None, delivery_ids=None):
        return self.validate_batch(
            0,
            self.delivery_ids if delivery_ids is None else delivery_ids,
            self.batch if batch is None else batch,
            owner="forge-admin",
            repository="agent-forge-core",
            issue_number=1,
            current_issue=self.issue,
        )

    def test_accepts_two_replayed_deliveries_bound_to_one_canonical_issue_read(self) -> None:
        checked = self._validate()
        self.assertEqual([row["delivery_id"] for row in checked], self.delivery_ids)
        self.assertEqual(len(checked), 2)

    def test_rejects_reordered_or_substituted_delivery_results(self) -> None:
        batch = copy.deepcopy(self.batch)
        batch["results"].reverse()
        with self.assertRaisesRegex(ValueError, "delivery ID"):
            self._validate(batch)

    def test_rejects_more_than_one_upstream_read_for_the_duplicate_resource(self) -> None:
        batch = copy.deepcopy(self.batch)
        batch["upstream_read_attempts"] = 2
        with self.assertRaisesRegex(ValueError, "one upstream read"):
            self._validate(batch)

    def test_rejects_canonical_state_that_differs_from_the_live_issue_readback(self) -> None:
        batch = copy.deepcopy(self.batch)
        batch["results"][1]["canonical_state"]["state"] = "closed"
        with self.assertRaisesRegex(ValueError, "current issue readback"):
            self._validate(batch)


class ReceiverOverloadBindingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.task = {
            "uuid": "delivery-overload",
            "is_delivered": True,
            "is_succeed": False,
            "response_status": 503,
        }
        self.proxy = {
            "delivery_id": "delivery-overload",
            "forwarded_status": 503,
            "forwarded_retry_after": "5",
            "request_body_sha256": "a" * 64,
        }
        self.drain = {
            "partial_worker_requests": 4,
            "partial_worker_statuses": [408, 408, 408, 408],
            "queued_probe_requests": 16,
            "queued_probe_statuses": [405] * 16,
            "drained": True,
        }

    def test_binds_forgejo_503_retry_after_and_drained_receiver_queue(self) -> None:
        result = driver.validate_overload_recovery(
            self.task, self.proxy, self.drain, []
        )
        self.assertEqual(result["forgejo_task_status"], 503)
        self.assertEqual(result["receiver_retry_after_seconds"], 5)
        self.assertIs(result["rejected_attempt_journaled"], False)

    def test_rejects_missing_retry_after_undrained_queue_or_journaled_failure(self) -> None:
        cases = []
        bad_proxy = copy.deepcopy(self.proxy)
        bad_proxy["forwarded_retry_after"] = None
        cases.append((bad_proxy, copy.deepcopy(self.drain), []))
        bad_drain = copy.deepcopy(self.drain)
        bad_drain["queued_probe_statuses"][-1] = 200
        cases.append((copy.deepcopy(self.proxy), bad_drain, []))
        cases.append((copy.deepcopy(self.proxy), copy.deepcopy(self.drain), [{"sequence": 1}]))
        for proxy, drain, records in cases:
            with self.subTest(proxy=proxy, drain=drain, records=records):
                with self.assertRaises(ValueError):
                    driver.validate_overload_recovery(
                        self.task, proxy, drain, records
                    )

class CountedReconciliationApiReadTests(unittest.TestCase):
    def setUp(self) -> None:
        validator = getattr(driver, "validate_counted_reconciliation_api_reads", None)
        self.assertTrue(callable(validator), "independent API read validator is missing")
        self.validate = validator
        self.requests = [
            {
                "method": "GET",
                "path": "/api/v1/repos/forge-admin/agent-forge-core/issues/1",
                "status": 200,
                "reader_credential_match": True,
            }
        ]

    def test_accepts_one_exact_read_only_issue_get_without_recording_credential(self) -> None:
        result = self.validate(
            self.requests, owner="forge-admin", repository="agent-forge-core", issue_number=1
        )
        self.assertEqual(result["request_count"], 1)
        self.assertEqual(result["credential_role"], "read_only_non_admin")
        self.assertNotIn("authorization", result["requests"][0])

    def test_rejects_retried_or_additional_upstream_requests(self) -> None:
        with self.assertRaisesRegex(ValueError, "exactly one"):
            self.validate(
                [*self.requests, *self.requests],
                owner="forge-admin",
                repository="agent-forge-core",
                issue_number=1,
            )

    def test_rejects_wrong_resource_status_method_or_reader_credential(self) -> None:
        mutations = [
            {"path": "/api/v1/repos/other/repo/issues/1"},
            {"status": 404},
            {"method": "POST"},
            {"reader_credential_match": False},
        ]
        for change in mutations:
            with self.subTest(change=change):
                request = {**self.requests[0], **change}
                with self.assertRaises(ValueError):
                    self.validate(
                        [request],
                        owner="forge-admin",
                        repository="agent-forge-core",
                        issue_number=1,
                    )


class MixedIssueReconciliationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.delivery_ids = ["missing-issue-delivery", "existing-issue-delivery"]
        self.resource_numbers = [2, 1]
        self.issue_one = {
            "number": 1,
            "state": "open",
            "updated_at": "2026-10-25T12:00:00-04:00",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/1",
        }
        self.issue_two = {
            "number": 2,
            "state": "open",
            "updated_at": "2026-10-25T12:05:00-04:00",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/2",
        }
        resource_one = {
            "kind": "issue", "owner": "forge-admin", "repository": "agent-forge-core", "number": 1
        }
        resource_two = {
            "kind": "issue", "owner": "forge-admin", "repository": "agent-forge-core", "number": 2
        }
        self.missing_batch = {
            "schema_version": 23,
            "api_version": "forgejo-rest-v1",
            "verdict": "inconclusive",
            "upstream_read_attempts": 2,
            "results": [
                {
                    "delivery_id": self.delivery_ids[0],
                    "verdict": "inconclusive",
                    "error": {
                        "code": "resource_not_found",
                        "retryable": False,
                        "details": {"resource": resource_two},
                    },
                },
                {
                    "delivery_id": self.delivery_ids[1],
                    "verdict": "current_state_read",
                    "resource": resource_one,
                    "canonical_state": {
                        "state": "open",
                        "updated_at": self.issue_one["updated_at"],
                        "html_url": self.issue_one["html_url"],
                    },
                },
            ],
        }
        self.recovered_batch = {
            "schema_version": 23,
            "api_version": "forgejo-rest-v1",
            "verdict": "current_state_read",
            "upstream_read_attempts": 2,
            "results": [
                {
                    "delivery_id": self.delivery_ids[0],
                    "verdict": "current_state_read",
                    "resource": resource_two,
                    "canonical_state": {
                        "state": "open",
                        "updated_at": self.issue_two["updated_at"],
                        "html_url": self.issue_two["html_url"],
                    },
                },
                {
                    "delivery_id": self.delivery_ids[1],
                    "verdict": "current_state_read",
                    "resource": resource_one,
                    "canonical_state": {
                        "state": "open",
                        "updated_at": self.issue_one["updated_at"],
                        "html_url": self.issue_one["html_url"],
                    },
                },
            ],
        }

    def _validate_batch(self, exit_status, batch, current_issues):
        return driver.validate_mixed_issue_reconciliation_batch(
            exit_status,
            self.delivery_ids,
            batch,
            owner="forge-admin",
            repository="agent-forge-core",
            resource_numbers=self.resource_numbers,
            current_issues=current_issues,
        )

    def test_accepts_first_resource_404_while_reading_distinct_sibling(self) -> None:
        rows = self._validate_batch(1, self.missing_batch, {1: self.issue_one})
        self.assertEqual([row["delivery_id"] for row in rows], self.delivery_ids)
        self.assertEqual(rows[0]["error"]["code"], "resource_not_found")
        self.assertEqual(rows[1]["canonical_state"]["state"], "open")

    def test_accepts_same_delivery_ids_after_missing_issue_is_created(self) -> None:
        rows = self._validate_batch(
            0, self.recovered_batch, {1: self.issue_one, 2: self.issue_two}
        )
        self.assertEqual([row["delivery_id"] for row in rows], self.delivery_ids)
        self.assertEqual(rows[0]["resource"]["number"], 2)
        self.assertEqual(rows[1]["resource"]["number"], 1)

    def test_rejects_a_cached_404_that_suppresses_a_sibling_or_survives_recovery(self) -> None:
        poisoned = copy.deepcopy(self.missing_batch)
        poisoned["results"][1] = copy.deepcopy(poisoned["results"][0])
        poisoned["results"][1]["delivery_id"] = self.delivery_ids[1]
        with self.assertRaisesRegex(ValueError, "existing issue result"):
            self._validate_batch(1, poisoned, {1: self.issue_one})

        stale = copy.deepcopy(self.recovered_batch)
        stale["results"][0] = copy.deepcopy(self.missing_batch["results"][0])
        with self.assertRaisesRegex(ValueError, "existing issue result"):
            self._validate_batch(0, stale, {1: self.issue_one, 2: self.issue_two})

    def test_rejects_reordered_results_and_wrong_read_count(self) -> None:
        reordered = copy.deepcopy(self.missing_batch)
        reordered["results"].reverse()
        overread = copy.deepcopy(self.missing_batch)
        overread["upstream_read_attempts"] = 3
        for batch in (reordered, overread):
            with self.subTest(batch=batch), self.assertRaises(ValueError):
                self._validate_batch(1, batch, {1: self.issue_one})

    def test_requires_two_ordered_authenticated_resource_specific_api_reads(self) -> None:
        requests = [
            {
                "method": "GET", "path": "/api/v1/repos/forge-admin/agent-forge-core/issues/2",
                "status": 404, "reader_credential_match": True,
            },
            {
                "method": "GET", "path": "/api/v1/repos/forge-admin/agent-forge-core/issues/1",
                "status": 200, "reader_credential_match": True,
            },
        ]
        checked = driver.validate_mixed_issue_api_reads(
            requests,
            owner="forge-admin",
            repository="agent-forge-core",
            resource_numbers=self.resource_numbers,
            expected_statuses=[404, 200],
        )
        self.assertEqual(checked["request_count"], 2)
        self.assertNotIn("authorization", checked["requests"][0])
        wrong_status = copy.deepcopy(requests)
        wrong_status[0]["status"] = 200
        with self.assertRaises(ValueError):
            driver.validate_mixed_issue_api_reads(
                wrong_status,
                owner="forge-admin",
                repository="agent-forge-core",
                resource_numbers=self.resource_numbers,
                expected_statuses=[404, 200],
            )


class NativeDeletedIssueBindingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.task = {
            "event_type": "issues",
            "uuid": "native-deleted-issue-delivery",
            "is_delivered": True,
            "is_succeed": True,
            "response_status": 202,
        }
        self.event = {
            "sequence": 5,
            "delivery_id": "native-deleted-issue-delivery",
            "event": "issues",
            "action": "opened",
            "issue_number": 3,
            "payload_state": "open",
        }

    def test_accepts_native_created_delivery_verified_absent_before_reconcile(self) -> None:
        validator = getattr(driver, "validate_native_deleted_issue_binding", None)
        self.assertTrue(callable(validator), "native deleted-issue binding validator is missing")
        checked = validator(self.task, self.event, issue_number=3, owner_readback_status=404)
        self.assertEqual(checked["delivery_id"], self.task["uuid"])
        self.assertEqual(checked["issue_number"], 3)
        self.assertEqual(checked["owner_readback_status"], 404)

    def test_rejects_unbound_or_non_native_or_still_existing_issue_evidence(self) -> None:
        validator = getattr(driver, "validate_native_deleted_issue_binding", None)
        self.assertTrue(callable(validator), "native deleted-issue binding validator is missing")
        mutations = (
            ({**self.task, "uuid": "different-delivery"}, self.event, 404),
            ({**self.task, "response_status": 500}, self.event, 404),
            (self.task, {**self.event, "action": "edited"}, 404),
            (self.task, {**self.event, "issue_number": 4}, 404),
            (self.task, self.event, 200),
        )
        for task, event, owner_status in mutations:
            with self.subTest(task=task, event=event, owner_status=owner_status):
                with self.assertRaises(ValueError):
                    validator(task, event, issue_number=3, owner_readback_status=owner_status)


class PrivateNotFoundReconciliationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.delivery_ids = ["native-replay-001", "native-replay-002"]
        self.batch = {
            "schema_version": 23,
            "api_version": "forgejo-rest-v1",
            "verdict": "inconclusive",
            "upstream_read_attempts": 1,
            "results": [
                {
                    "verdict": "inconclusive",
                    "delivery_id": delivery_id,
                    "error": {
                        "code": "resource_not_found",
                        "retryable": False,
                        "action": "verify issue number and repository access; HTTP 404 does not distinguish a missing issue from an inaccessible repository",
                        "details": {
                            "upstream": "forgejo",
                            "resource": {
                                "kind": "issue",
                                "owner": "forge-admin",
                                "repository": "agent-forge-core",
                                "number": 1,
                            },
                        },
                    },
                }
                for delivery_id in self.delivery_ids
            ],
        }
        self.request = {
            "method": "GET",
            "path": "/api/v1/repos/forge-admin/agent-forge-core/issues/1",
            "status": 404,
            "reader_credential_match": True,
        }

    def test_accepts_real_not_found_as_inconclusive_and_nonretryable(self) -> None:
        validate_batch = getattr(
            driver, "validate_private_not_found_reconciliation_batch", None
        )
        validate_reads = getattr(driver, "validate_counted_private_not_found_reads", None)
        self.assertTrue(callable(validate_batch), "private 404 batch validator is missing")
        self.assertTrue(callable(validate_reads), "private 404 API validator is missing")
        rows = validate_batch(
            1,
            self.delivery_ids,
            self.batch,
            owner="forge-admin",
            repository="agent-forge-core",
            issue_number=1,
        )
        read = validate_reads(
            [self.request],
            owner="forge-admin",
            repository="agent-forge-core",
            issue_number=1,
        )
        self.assertEqual([row["delivery_id"] for row in rows], self.delivery_ids)
        self.assertEqual(read["requests"][0]["status"], 404)
        self.assertIs(rows[0]["error"]["retryable"], False)

    def test_rejects_success_or_retryable_result_after_private_404(self) -> None:
        validate = getattr(driver, "validate_private_not_found_reconciliation_batch", None)
        self.assertTrue(callable(validate), "private 404 batch validator is missing")
        with self.assertRaises(ValueError):
            validate(
                0,
                self.delivery_ids,
                self.batch,
                owner="forge-admin",
                repository="agent-forge-core",
                issue_number=1,
            )
        retryable = copy.deepcopy(self.batch)
        retryable["results"][0]["error"]["retryable"] = True
        with self.assertRaises(ValueError):
            validate(
                1,
                self.delivery_ids,
                retryable,
                owner="forge-admin",
                repository="agent-forge-core",
                issue_number=1,
            )

    def test_rejects_missing_error_rows_or_reordered_delivery_ids(self) -> None:
        validate = getattr(driver, "validate_private_not_found_reconciliation_batch", None)
        self.assertTrue(callable(validate), "private 404 batch validator is missing")
        missing = copy.deepcopy(self.batch)
        del missing["results"][0]["error"]
        reordered = copy.deepcopy(self.batch)
        reordered["results"].reverse()
        for batch in (missing, reordered):
            with self.subTest(batch=batch), self.assertRaises(ValueError):
                validate(
                    1,
                    self.delivery_ids,
                    batch,
                    owner="forge-admin",
                    repository="agent-forge-core",
                    issue_number=1,
                )

    def test_rejects_a_retried_or_unbound_404_observation(self) -> None:
        validate = getattr(driver, "validate_counted_private_not_found_reads", None)
        self.assertTrue(callable(validate), "private 404 API validator is missing")
        for requests in (
            [self.request, self.request],
            [{**self.request, "reader_credential_match": False}],
            [{**self.request, "status": 200}],
        ):
            with self.subTest(requests=requests), self.assertRaises(ValueError):
                validate(
                    requests,
                    owner="forge-admin",
                    repository="agent-forge-core",
                    issue_number=1,
                )


if __name__ == "__main__":
    unittest.main()
