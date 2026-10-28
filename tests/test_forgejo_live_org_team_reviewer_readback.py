import sys
import tempfile
import threading
import unittest
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from run_forgejo_live_org_team_reviewer_readback import (
    build_receive_pack_request_body,
    enable_console_info_access_logs,
    validate_review_requested_delivery_record,
    validate_read_only_git_receive_pack_probe,
    validate_read_only_write_scope_probe,
    validate_team_readback,
    validate_write_credential_receive_pack_control,
    validate_pr_comment_batch_result,
    validate_pr_comment_batch_api_reads,
    validate_pr_comment_batch_receiver_restart,
    validate_interrupted_cli_attempt,
    validate_interrupted_cli_resume,
)
from run_forgejo_live_webhook_reconcile_e2e import normalize_delivery_ids
from run_forgejo_live_webhook_native_replay_e2e import OneShotApiGetBarrier


class OneShotApiGetBarrierTests(unittest.TestCase):
    def test_blocks_only_the_bound_get_then_releases_it_for_forwarding(self):
        barrier = OneShotApiGetBarrier(
            "/api/v1/repos/forge-team-org/team-review-readback/issues/comments/7"
        )
        self.assertFalse(barrier.claim("POST", "/api/v1/repos/forge-team-org/team-review-readback/issues/comments/7"))
        self.assertFalse(barrier.claim("GET", "/api/v1/repos/forge-team-org/team-review-readback/issues/comments/8"))
        self.assertTrue(
            barrier.claim("GET", "/api/v1/repos/forge-team-org/team-review-readback/issues/comments/7")
        )

        outcome = []
        waiter = threading.Thread(target=lambda: outcome.append(barrier.wait_for_decision()))
        waiter.start()
        self.assertTrue(barrier.wait_until_reached(1))
        self.assertTrue(barrier.release(forward=True))
        self.assertTrue(barrier.wait_until_finished(1))
        waiter.join(timeout=1)
        self.assertEqual(outcome, [True])
        self.assertFalse(
            barrier.claim("GET", "/api/v1/repos/forge-team-org/team-review-readback/issues/comments/7")
        )

    def test_drops_the_claimed_get_after_the_caller_cancels_the_cli(self):
        barrier = OneShotApiGetBarrier(
            "/api/v1/repos/forge-team-org/team-review-readback/issues/comments/7"
        )
        self.assertTrue(
            barrier.claim("GET", "/api/v1/repos/forge-team-org/team-review-readback/issues/comments/7")
        )
        outcome = []
        waiter = threading.Thread(target=lambda: outcome.append(barrier.wait_for_decision()))
        waiter.start()
        self.assertTrue(barrier.wait_until_reached(1))
        self.assertTrue(barrier.release(forward=False))
        self.assertTrue(barrier.wait_until_finished(1))
        waiter.join(timeout=1)
        self.assertEqual(outcome, [False])


class ValidateInterruptedReconciliationTests(unittest.TestCase):
    parent_path = "/api/v1/repos/forge-team-org/team-review-readback/pulls/1"
    comment_path = "/api/v1/repos/forge-team-org/team-review-readback/issues/comments/7"

    @staticmethod
    def request(method, path, status=200, *, forwarded=True, barrier="none"):
        return {
            "method": method,
            "path": path,
            "status": status,
            "reader_credential_match": True,
            "forwarded": forwarded,
            "gate": "allowlisted",
            "barrier": barrier,
        }

    def first_attempt(self):
        return validate_interrupted_cli_attempt(
            proxy_records=[
                self.request("GET", self.parent_path),
                self.request(
                    "GET",
                    self.comment_path,
                    503,
                    forwarded=False,
                    barrier="cancelled_before_upstream",
                ),
            ],
            parent_get_path=self.parent_path,
            comment_get_path=self.comment_path,
            server_log_delta=(
                "[I] completed GET " + self.parent_path + " 200 OK in 1ms\n"
            ),
            journal_before=b"canonical-inbox-bytes\n",
            journal_after=b"canonical-inbox-bytes\n",
            barrier={
                "path": self.comment_path,
                "claimed": True,
                "reached": True,
                "released": True,
                "forward": False,
                "finished": True,
            },
            killed_process={
                "returncode": -9,
                "signal": 9,
                "group_gone": True,
                "credential_exposed": False,
            },
            forbidden_route_record={
                "method": "POST",
                "status": 403,
                "forwarded": False,
                "gate": "allowlisted",
                "barrier": "not_allowlisted",
                "reader_credential_match": True,
            },
            forbidden_status=403,
            forbidden_post_upstream_hits=[],
        )

    def test_accepts_parent_read_then_cancelled_comment_read_before_upstream(self):
        summary = self.first_attempt()
        self.assertTrue(summary["parent_get_completed_before_interruption"])
        self.assertTrue(summary["comment_get_held_then_cancelled_before_upstream"])
        self.assertTrue(summary["journal_unchanged_after_kill"])

    def test_rejects_forwarded_comment_read_changed_journal_or_live_child(self):
        base = self.first_attempt()
        proxy_records = base["proxy_records"]
        proxy_records[1]["forwarded"] = True
        proxy_records[1]["barrier"] = "released_to_upstream"
        with self.assertRaisesRegex(RuntimeError, "cancel the comment GET"):
            validate_interrupted_cli_attempt(
                proxy_records=proxy_records,
                parent_get_path=self.parent_path,
                comment_get_path=self.comment_path,
                server_log_delta=("completed GET " + self.parent_path + " 200 OK\n"),
                journal_before=b"canonical-inbox-bytes\n",
                journal_after=b"canonical-inbox-bytes\n",
                barrier=base["barrier"],
                killed_process=base["killed_process"],
                forbidden_route_record=base["forbidden_route_record"],
                forbidden_status=403,
                forbidden_post_upstream_hits=[],
            )
        base = self.first_attempt()
        with self.assertRaisesRegex(RuntimeError, "preserve its evidence"):
            validate_interrupted_cli_attempt(
                proxy_records=base["proxy_records"],
                parent_get_path=self.parent_path,
                comment_get_path=self.comment_path,
                server_log_delta=("completed GET " + self.parent_path + " 200 OK\n"),
                journal_before=b"old-inbox\n",
                journal_after=b"changed-inbox\n",
                barrier=base["barrier"],
                killed_process=base["killed_process"] | {"group_gone": False},
                forbidden_route_record=base["forbidden_route_record"],
                forbidden_status=403,
                forbidden_post_upstream_hits=[],
            )

    def test_accepts_resume_of_same_batch_and_refuses_wrong_order_or_missing_evidence(self):
        records = [
            self.request("GET", self.parent_path),
            self.request("GET", self.comment_path),
        ]
        summary = validate_interrupted_cli_resume(
            proxy_records=records,
            parent_get_path=self.parent_path,
            comment_get_path=self.comment_path,
            reconciled={
                "schema_version": 23,
                "verdict": "current_state_read",
                "upstream_read_attempts": 2,
            },
            journal_before=b"canonical-inbox-bytes\n",
            journal_after=b"canonical-inbox-bytes\n",
        )
        self.assertTrue(summary["resumed_cli_completed_same_two_delivery_batch"])
        records.reverse()
        with self.assertRaisesRegex(RuntimeError, "allowlist"):
            validate_interrupted_cli_resume(
                proxy_records=records,
                parent_get_path=self.parent_path,
                comment_get_path=self.comment_path,
                reconciled={
                    "schema_version": 23,
                    "verdict": "current_state_read",
                    "upstream_read_attempts": 2,
                },
                journal_before=b"canonical-inbox-bytes\n",
                journal_after=b"canonical-inbox-bytes\n",
            )


class ReceivePackRequestBodyTests(unittest.TestCase):
    def test_encodes_a_single_new_ref_command_followed_by_a_git_pack(self):
        old_oid = "0" * 40
        new_oid = "1" * 40
        pack = b"PACK" + b"synthetic-pack-body"

        body = build_receive_pack_request_body(
            old_oid=old_oid,
            new_oid=new_oid,
            ref="refs/heads/agent/probe",
            pack=pack,
        )

        command = f"{old_oid} {new_oid} refs/heads/agent/probe".encode() + b"\x00report-status\n"
        packet = f"{len(command) + 4:04x}".encode() + command
        self.assertEqual(body, packet + b"0000" + pack)

    def test_rejects_malformed_oids_refs_and_non_git_pack(self):
        valid = {
            "old_oid": "0" * 40,
            "new_oid": "1" * 40,
            "ref": "refs/heads/agent/probe",
            "pack": b"PACKsynthetic-pack-body",
        }
        for field, value in (
            ("new_oid", "not-an-oid"),
            ("ref", "refs/heads/bad ref"),
            ("pack", b"not-a-pack"),
        ):
            with self.subTest(field=field), self.assertRaises(ValueError):
                build_receive_pack_request_body(**(valid | {field: value}))


class ForgejoAccessLogProbeConfigurationTests(unittest.TestCase):
    def test_enables_console_info_logs_only_for_the_private_probe_config(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            config_path = state / "custom/conf/app.ini"
            config_path.parent.mkdir(parents=True)
            original = (
                b"[security]\nSECRET_KEY = synthetic-private-fixture\n\n"
                b"[log]\nMODE = file\nLEVEL = Warn\nROOT_PATH = /private/log\n\n"
                b"[actions]\nENABLED = false\n"
            )
            config_path.write_bytes(original)

            enable_console_info_access_logs(state)

            self.assertEqual(
                config_path.read_bytes(),
                original.replace(
                    b"[log]\nMODE = file\nLEVEL = Warn\n",
                    b"[log]\nMODE = console\nLEVEL = Info\n",
                    1,
                ),
            )
            self.assertEqual(config_path.stat().st_mode & 0o777, 0o600)


class ValidateTeamReadbackTests(unittest.TestCase):
    def test_accepts_exact_nonempty_team_projection_after_created_request(self):
        projection = validate_team_readback(
            request_status=201,
            pull_request={
                "requested_reviewers_teams": [{"id": 12, "name": "reviewers"}]
            },
            expected_team_id=12,
            expected_team_name="reviewers",
        )
        self.assertEqual(projection, [{"id": 12, "name": "reviewers"}])

    def test_rejects_empty_or_missing_live_team_readback(self):
        for pull_request in (
            {"requested_reviewers_teams": []},
            {},
        ):
            with self.subTest(pull_request=pull_request):
                with self.assertRaisesRegex(RuntimeError, "did not contain the requested team"):
                    validate_team_readback(
                        request_status=201,
                        pull_request=pull_request,
                        expected_team_id=12,
                        expected_team_name="reviewers",
                    )

    def test_rejects_noncreated_request(self):
        with self.assertRaisesRegex(RuntimeError, "HTTP 201"):
            validate_team_readback(
                request_status=200,
                pull_request={
                    "requested_reviewers_teams": [{"id": 12, "name": "reviewers"}]
                },
                expected_team_id=12,
                expected_team_name="reviewers",
            )

    def test_rejects_different_team_identity(self):
        with self.assertRaisesRegex(RuntimeError, "did not contain the requested team"):
            validate_team_readback(
                request_status=201,
                pull_request={
                    "requested_reviewers_teams": [{"id": 13, "name": "other-team"}]
                },
                expected_team_id=12,
                expected_team_name="reviewers",
            )


class ValidateReviewRequestedDeliveryTests(unittest.TestCase):
    @staticmethod
    def record():
        delivery_id = "00000000-0000-4000-8000-000000000001"
        return {
            "sequence": 1,
            "kind": "forgejo.webhook.v1.pull_request",
            "idempotency_key": f"forgejo-delivery-v1:{delivery_id}",
            "payload": {
                "schema_version": 1,
                "provider": "forgejo",
                "event": "pull_request",
                "delivery_id": delivery_id,
                "payload": {
                    "action": "review_requested",
                    "repository": {"full_name": "forge-team-org/team-review-readback"},
                    "requested_team": {"id": 12, "name": "reviewers"},
                    "pull_request": {
                        "number": 3,
                        "state": "open",
                        "merged": False,
                        "head": {"sha": "a" * 40},
                        "base": {"sha": "b" * 40},
                    },
                },
            },
        }

    def test_accepts_a_bound_review_requested_team_event(self):
        summary = validate_review_requested_delivery_record(
            self.record(),
            owner="forge-team-org",
            repository="team-review-readback",
            pull_request_number=3,
        )
        self.assertEqual(summary["delivery_id"], "00000000-0000-4000-8000-000000000001")
        self.assertEqual(summary["action"], "review_requested")
        self.assertEqual(summary["requested_team"], {"id": 12, "name": "reviewers"})

    def test_rejects_unrelated_or_malformed_delivery_bindings(self):
        mutations = (
            ("wrong kind", lambda r: r.update(kind="forgejo.webhook.v1.issues")),
            (
                "wrong action",
                lambda r: r["payload"]["payload"].update(action="opened"),
            ),
            (
                "wrong repository",
                lambda r: r["payload"]["payload"]["repository"].update(
                    full_name="elsewhere/private"
                ),
            ),
            (
                "wrong PR number",
                lambda r: r["payload"]["payload"]["pull_request"].update(number=4),
            ),
        )
        for name, mutate in mutations:
            record = self.record()
            mutate(record)
            with self.subTest(name=name), self.assertRaises(ValueError):
                validate_review_requested_delivery_record(
                    record,
                    owner="forge-team-org",
                    repository="team-review-readback",
                    pull_request_number=3,
                )


class ValidateReadOnlyWriteScopeProbeTests(unittest.TestCase):
    @staticmethod
    def pull_request():
        return {
            "number": 3,
            "title": "Verify organization team reviewer readback",
            "body": "Disposable local API integration fixture.",
            "state": "open",
            "merged": False,
            "base": {
                "ref": "main",
                "sha": "b" * 40,
                "repo": {"full_name": "forge-team-org/team-review-readback"},
            },
            "head": {"ref": "agent/team-review", "sha": "a" * 40},
            "requested_reviewers": [],
            "requested_reviewers_teams": [{"id": 12, "name": "reviewers"}],
            "updated_at": "2026-10-27T00:00:00Z",
        }

    def test_accepts_403_when_comment_list_and_pr_binding_are_unchanged(self):
        summary = validate_read_only_write_scope_probe(
            http_status=403,
            marker="scope-probe-marker",
            comments_before=[],
            comments_after=[],
            pull_request_before=self.pull_request(),
            pull_request_after=self.pull_request(),
        )
        self.assertEqual(summary["http_status"], 403)
        self.assertTrue(summary["comments_unchanged"])
        self.assertTrue(summary["pull_request_binding_unchanged"])

    def test_rejects_success_or_any_observed_write_effect(self):
        created_comment = [{"id": 9, "body": "scope-probe-marker"}]
        changed_pr = self.pull_request()
        changed_pr["title"] = "Unexpectedly changed"
        cases = (
            (201, [], created_comment, self.pull_request()),
            (403, [], created_comment, self.pull_request()),
            (403, [], [], changed_pr),
            (404, [], [], self.pull_request()),
        )
        for status, before, after, pr_after in cases:
            with self.subTest(status=status, after=after, pr_after=pr_after), self.assertRaises(
                RuntimeError
            ):
                validate_read_only_write_scope_probe(
                    http_status=status,
                    marker="scope-probe-marker",
                    comments_before=before,
                    comments_after=after,
                    pull_request_before=self.pull_request(),
                    pull_request_after=pr_after,
                )


class ValidateReadOnlyGitReceivePackProbeTests(unittest.TestCase):
    @staticmethod
    def arguments():
        token = "synthetic-read-only-token"
        return {
            "read_exit_code": 0,
            "read_stdout": "0123456789abcdef\trefs/heads/main\n",
            "read_stderr": "",
            "push_exit_code": 128,
            "push_stdout": "",
            "push_stderr": "remote: HTTP 403 Forbidden\nfatal: remote error: access denied\n",
            "empty_flush_post_status": 403,
            "valid_push_post_status": 403,
            "target_branch": "agent/read-only-git-write-probe",
            "branches_before": [
                {"name": "main"},
                {"name": "agent/team-review"},
            ],
            "branches_after": [
                {"name": "main"},
                {"name": "agent/team-review"},
            ],
            "server_log": (
                "router: completed GET /forge-team-org/team-review-readback.git/"
                "info/refs?service=git-receive-pack for 127.0.0.1:3130, 403 Forbidden in 1ms\n"
                "router: completed POST "
                "/forge-team-org/team-review-readback.git/git-receive-pack "
                "for 127.0.0.1:3130, 403 Forbidden in 1ms\n"
                "router: completed POST "
                "/forge-team-org/team-review-readback.git/git-receive-pack "
                "for 127.0.0.1:3130, 403 Forbidden in 1ms"
            ),
            "token": token,
        }

    def test_accepts_authenticated_git_read_and_denied_receive_pack_without_ref_change(self):
        summary = validate_read_only_git_receive_pack_probe(**self.arguments())
        self.assertTrue(summary["authenticated_git_read_confirmed"])
        self.assertEqual(summary["receive_pack_advertisement_http_status"], 403)
        self.assertEqual(summary["empty_flush_post_http_status"], 403)
        self.assertEqual(summary["valid_push_post_http_status"], 403)
        self.assertTrue(summary["target_ref_absent_before"])
        self.assertTrue(summary["target_ref_absent_after"])
        self.assertTrue(summary["credential_absent_from_git_output"])

    def test_rejects_success_missing_http_denial_changed_ref_or_credential_leak(self):
        mutations = (
            ("push succeeded", {"push_exit_code": 0}),
            ("empty direct POST succeeded", {"empty_flush_post_status": 200}),
            ("valid push POST succeeded", {"valid_push_post_status": 200}),
            (
                "valid POST denial missing despite denied advertisement",
                {
                    "server_log": "router: completed GET /forge-team-org/team-review-readback.git/info/refs?service=git-receive-pack for 127.0.0.1:3130, 403 Forbidden in 1ms"
                },
            ),
            (
                "only one direct POST logged",
                {
                    "server_log": (
                        "router: completed GET /forge-team-org/team-review-readback.git/info/refs?service=git-receive-pack for 127.0.0.1:3130, 403 Forbidden in 1ms\n"
                        "router: completed POST /forge-team-org/team-review-readback.git/git-receive-pack for 127.0.0.1:3130, 403 Forbidden in 1ms"
                    )
                },
            ),
            (
                "unrelated POST denied",
                {
                    "server_log": "router: completed POST /forge-team-org/other.git/git-receive-pack for 127.0.0.1:3130, 403 Forbidden in 1ms"
                },
            ),
            (
                "target ref created",
                {"branches_after": [*self.arguments()["branches_before"], {"name": "agent/read-only-git-write-probe"}]},
            ),
            ("missing server denial", {"server_log": "unrelated route returned 403 Forbidden"}),
            ("read access failed", {"read_exit_code": 128, "read_stdout": ""}),
            ("credential leaked", {"push_stderr": "token synthetic-read-only-token"}),
        )
        for name, mutation in mutations:
            with self.subTest(name=name), self.assertRaises(RuntimeError):
                validate_read_only_git_receive_pack_probe(**(self.arguments() | mutation))


class ValidateWriteCredentialReceivePackControlTests(unittest.TestCase):
    @staticmethod
    def arguments():
        branch = "agent/read-write-control-probe"
        oid = "d" * 40
        status_line = f"ok refs/heads/{branch}\n".encode()
        packet = f"{len(status_line) + 4:04x}".encode() + status_line
        return {
            "post_status": 200,
            "response_body": b"000eunpack ok\n" + packet + b"0000",
            "control_branch": branch,
            "expected_oid": oid,
            "branches_before": [{"name": "main"}],
            "branches_after": [
                {"name": "main"},
                {"name": branch, "commit": {"id": oid}},
            ],
            "server_log": (
                "router: completed POST "
                "/forge-team-org/team-review-readback.git/git-receive-pack "
                "for 127.0.0.1:3130, 200 OK in 2ms"
            ),
        }

    def test_accepts_report_status_and_current_branch_readback_for_writer_control(self):
        result = validate_write_credential_receive_pack_control(**self.arguments())
        self.assertTrue(result["write_effect_confirmed"])
        self.assertTrue(result["target_branch_readback_matches"])
        self.assertTrue(result["receive_pack_report_status_ok"])

    def test_rejects_missing_success_report_route_or_ref_readback(self):
        arguments = self.arguments()
        mutations = (
            ("not accepted", {"post_status": 403}),
            ("missing report status", {"response_body": b"0000"}),
            ("wrong ref readback", {"branches_after": [{"name": "main"}]}),
            (
                "wrong oid readback",
                {"branches_after": [{"name": "agent/read-write-control-probe", "commit": {"id": "e" * 40}}]},
            ),
            ("branch existed before", {"branches_before": [{"name": "agent/read-write-control-probe"}]}),
            ("missing route status", {"server_log": "completed POST /other/repo.git/git-receive-pack 200 OK"}),
        )
        for name, mutation in mutations:
            with self.subTest(name=name), self.assertRaises(RuntimeError):
                validate_write_credential_receive_pack_control(**(arguments | mutation))

if __name__ == "__main__":
    unittest.main()


class NormalizeReconcileDeliveryIdsTests(unittest.TestCase):
    def test_preserves_a_single_delivery_id_as_one_argument(self):
        self.assertEqual(normalize_delivery_ids("one-id"), ("one-id",))

    def test_preserves_batch_delivery_order_as_separate_arguments(self):
        self.assertEqual(normalize_delivery_ids(["pr-id", "comment-id"]), ("pr-id", "comment-id"))

    def test_rejects_empty_or_non_string_batch_members(self):
        for deliveries in ([], ["pr-id", ""], ["pr-id", 4], b"not-delivery-ids"):
            with self.subTest(deliveries=deliveries), self.assertRaises(ValueError):
                normalize_delivery_ids(deliveries)


class ValidatePullRequestCommentBatchResultTests(unittest.TestCase):
    @staticmethod
    def result():
        pr_id = "pr-delivery-001"
        comment_delivery = "comment-delivery-002"
        owner = "forge-team-org"
        repository = "team-review-readback"
        number = 3
        comment_id = 915
        head = "a" * 40
        base = "b" * 40
        team = [{"id": 12, "name": "reviewers"}]
        resource = {
            "kind": "pull_request",
            "owner": owner,
            "repository": repository,
            "number": number,
        }
        current = {
            "state": "open",
            "merged": False,
            "head_sha": head,
            "base_sha": base,
            "requested_reviewers_teams": team,
            "comment": None,
        }
        comment_current = current | {
            "comment": {
                "id": comment_id,
                "body": "synthetic live PR comment",
                "body_trust": "untrusted_user_content",
            }
        }
        return {
            "arguments": {
                "pull_delivery_id": pr_id,
                "comment_delivery_id": comment_delivery,
                "owner": owner,
                "repository": repository,
                "pull_request_number": number,
                "comment_id": comment_id,
                "expected_head_sha": head,
                "expected_base_sha": base,
                "expected_team_id": 12,
                "expected_team_name": "reviewers",
                "expected_comment_body": "synthetic live PR comment",
            },
            "value": {
                "schema_version": 23,
                "verdict": "current_state_read",
                "upstream_read_attempts": 2,
                "results": [
                    {
                        "delivery_id": pr_id,
                        "verdict": "current_state_read",
                        "resource": resource,
                        "canonical_state": current,
                    },
                    {
                        "delivery_id": comment_delivery,
                        "verdict": "current_state_read",
                        "resource": resource,
                        "canonical_state": comment_current,
                    },
                ],
            },
        }

    def test_accepts_two_native_deliveries_with_shared_current_pr_and_untrusted_comment(self):
        fixture = self.result()
        summary = validate_pr_comment_batch_result(fixture["value"], **fixture["arguments"])
        self.assertEqual(summary["delivery_ids"], ["pr-delivery-001", "comment-delivery-002"])
        self.assertEqual(summary["upstream_read_attempts"], 2)
        self.assertTrue(summary["shared_current_parent_confirmed"])
        self.assertEqual(summary["comment_body_trust"], "untrusted_user_content")

    def test_rejects_reordered_stale_missing_team_or_trusted_comment_batch(self):
        valid = self.result()
        mutations = (
            ("wrong delivery order", lambda value: value["results"].reverse()),
            ("extra upstream read", lambda value: value.update(upstream_read_attempts=3)),
            ("wrong parent", lambda value: value["results"][1]["resource"].update(number=4)),
            ("missing team", lambda value: value["results"][1]["canonical_state"].update(requested_reviewers_teams=[])),
            ("trusted comment", lambda value: value["results"][1]["canonical_state"]["comment"].update(body_trust="trusted")),
            ("stale comment body", lambda value: value["results"][1]["canonical_state"]["comment"].update(body="old text")),
        )
        for name, mutate in mutations:
            fixture = self.result()
            mutate(fixture["value"])
            with self.subTest(name=name), self.assertRaises(RuntimeError):
                validate_pr_comment_batch_result(fixture["value"], **fixture["arguments"])


class ValidatePullRequestCommentBatchApiReadTests(unittest.TestCase):
    @staticmethod
    def log():
        return (
            "router: completed GET /api/v1/repos/forge-team-org/team-review-readback/pulls/3 "
            "for 127.0.0.1:3130, 200 OK in 1ms\n"
            "router: completed GET /api/v1/repos/forge-team-org/team-review-readback/issues/comments/915 "
            "for 127.0.0.1:3130, 200 OK in 1ms\n"
        )

    def test_requires_one_successful_current_pr_and_comment_get(self):
        summary = validate_pr_comment_batch_api_reads(
            self.log(),
            owner="forge-team-org",
            repository="team-review-readback",
            pull_request_number=3,
            comment_id=915,
        )
        self.assertEqual(summary["upstream_get_count"], 2)
        self.assertTrue(summary["pull_request_get_confirmed"])
        self.assertTrue(summary["comment_get_confirmed"])

    def test_rejects_missing_duplicate_foreign_or_unsuccessful_gets(self):
        valid = self.log()
        mutations = (
            ("comment missing", valid.splitlines()[0] + "\n"),
            ("duplicate PR read", valid + valid.splitlines()[0] + "\n"),
            ("foreign repository", valid.replace("team-review-readback/issues/comments", "other/issues/comments")),
            ("comment not found", valid.replace("200 OK", "404 Not Found", 1)),
        )
        for name, log in mutations:
            with self.subTest(name=name), self.assertRaises(RuntimeError):
                validate_pr_comment_batch_api_reads(
                    log,
                    owner="forge-team-org",
                    repository="team-review-readback",
                    pull_request_number=3,
                    comment_id=915,
                )


class ValidatePullRequestCommentBatchReceiverRestartTests(unittest.TestCase):
    @staticmethod
    def fixture():
        delivery_ids = [
            "00000000-0000-4000-8000-000000000001",
            "00000000-0000-4000-8000-000000000002",
        ]
        records = [
            {
                "sequence": 1,
                "kind": "forgejo.webhook.v1.pull_request",
                "idempotency_key": f"forgejo-delivery-v1:{delivery_ids[0]}",
                "payload": {"event": "pull_request", "delivery_id": delivery_ids[0]},
            },
            {
                "sequence": 2,
                "kind": "forgejo.webhook.v1.issue_comment",
                "idempotency_key": f"forgejo-delivery-v1:{delivery_ids[1]}",
                "payload": {"event": "issue_comment", "delivery_id": delivery_ids[1]},
            },
        ]
        tasks = [
            {
                "hook_id": 7,
                "uuid": delivery_ids[0],
                "event_type": "pull_request_review_request",
                "is_delivered": True,
                "is_succeed": True,
                "response_status": 202,
            },
            {
                "hook_id": 7,
                "uuid": delivery_ids[1],
                "event_type": "pull_request_comment",
                "is_delivered": True,
                "is_succeed": True,
                "response_status": 202,
            },
        ]
        return {
            "journal_before": b"durable-native-event-1\ndurable-native-event-2\n",
            "journal_after": b"durable-native-event-1\ndurable-native-event-2\n",
            "records_before": records,
            "records_after": records,
            "hook_tasks": tasks,
            "hook_id": 7,
            "delivery_ids": delivery_ids,
            "killed_returncode": -9,
            "listeners_after_kill": [],
            "restarted_listener_owned": True,
        }

    def test_accepts_hard_restart_after_two_native_hook_acks_with_same_journal(self):
        summary = validate_pr_comment_batch_receiver_restart(**self.fixture())
        self.assertEqual(summary["event_types"], ["pull_request", "issue_comment"])
        self.assertEqual(summary["delivery_ids"], self.fixture()["delivery_ids"])
        self.assertTrue(summary["journal_byte_identical_after_restart"])
        self.assertTrue(summary["both_hook_tasks_acked_202_before_kill"])
        self.assertEqual(summary["receiver_kill_exit_code"], -9)

    def test_rejects_changed_journal_reordered_event_unacked_task_or_live_old_listener(self):
        mutations = (
            ("journal changed", {"journal_after": b"different journal"}),
            ("record order", {"records_after": list(reversed(self.fixture()["records_before"]))}),
            (
                "unacknowledged task",
                {"hook_tasks": self.fixture()["hook_tasks"][:1] + [{
                    **self.fixture()["hook_tasks"][1], "response_status": 500
                }]},
            ),
            ("old listener", {"listeners_after_kill": ["127.0.0.1:4444"]}),
            ("restart not owned", {"restarted_listener_owned": False}),
            ("not hard killed", {"killed_returncode": 0}),
            (
                "internal event type mismatch",
                {"hook_tasks": self.fixture()["hook_tasks"][:1] + [{
                    **self.fixture()["hook_tasks"][1], "event_type": "issue_comment"
                }]},
            ),
        )
        for name, mutation in mutations:
            fixture = self.fixture() | mutation
            with self.subTest(name=name), self.assertRaises(RuntimeError):
                validate_pr_comment_batch_receiver_restart(**fixture)
