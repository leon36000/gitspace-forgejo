"""Process-level check for the fixed host policy and schema-v3 admission path.

This test builds a temporary copy containing only Cargo.toml, Cargo.lock and
src/. Its compile-time CARGO_MANIFEST_DIR therefore points at the temporary
fixture, never the product checkout's private .runtime directory.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit


ROOT = Path(__file__).resolve().parents[1]
ORIGIN = ""
OWNER = "forge-admin"
REPOSITORY = "agent-forge-core"
BASE = "1111111111111111111111111111111111111111"
HEAD = "2222222222222222222222222222222222222222"
MERGE = "3333333333333333333333333333333333333333"
REQUIREMENTS = (
    b'{"schema_version":3,"id":"REQ-001","revision":1,'
    b'"source_issue_number":41,"source_pull_request_number":77,'
    b'"request":"Implement a duration parser.",'
    b'"acceptance":["250ms parses to 250 milliseconds."],'
    b'"ambiguities":[],"decisions":[{"ambiguity":"Bare integer units",'
    b'"resolution":"Bare integers mean milliseconds.",'
    b'"rationale":"This preserves the existing CLI convention."}]}\n'
)


class _Fixture:
    def __init__(self, source: bytes) -> None:
        self.source = source
        self.reviewer_id = 9001
        self.requests: list[str] = []
        self.lock = threading.Lock()

    def response(self, path: str) -> tuple[int, list[tuple[str, str]], bytes]:
        parsed = urlsplit(path)
        route = parsed.path
        if route == f"/api/v1/repos/{OWNER}/{REPOSITORY}/branches/main":
            return 200, [], json.dumps(
                {"name": "main", "protected": True, "commit": {"id": BASE}}
            ).encode()
        if route == "/api/v1/settings/api":
            return 200, [], b'{"default_git_trees_per_page":1000}'
        if route == f"/api/v1/repos/{OWNER}/{REPOSITORY}/issues/41":
            return 200, [], json.dumps(
                {
                    "number": 41,
                    "html_url": f"{ORIGIN}/{OWNER}/{REPOSITORY}/issues/41",
                    "state": "open",
                    "title": "Implement duration parsing",
                    "body": "Implement a duration parser.",
                    "updated_at": "2026-10-27T12:00:00Z",
                }
            ).encode()
        if route == f"/api/v1/repos/{OWNER}/{REPOSITORY}/git/trees/{BASE}":
            return 200, [], json.dumps(
                {
                    "page": 1,
                    "total_count": 2,
                    "truncated": False,
                    "tree": [
                        {"path": "requirements/REQ-001.json", "type": "blob", "mode": "100644"},
                        {"path": "src/lib.rs", "type": "blob", "mode": "100644"},
                    ],
                }
            ).encode()
        if route == f"/{OWNER}/{REPOSITORY}/raw/commit/{BASE}/requirements/REQ-001.json":
            return 200, [], REQUIREMENTS
        if route == f"/{OWNER}/{REPOSITORY}/raw/commit/{MERGE}/requirements/REQ-001.json":
            return 200, [], REQUIREMENTS
        if route == f"/{OWNER}/{REPOSITORY}/raw/commit/{BASE}/src/lib.rs":
            return 200, [], self.source
        if route == f"/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/77":
            return 200, [], json.dumps(
                {
                    "number": 77,
                    "html_url": f"{ORIGIN}/{OWNER}/{REPOSITORY}/pulls/77",
                    "state": "closed",
                    "merged": True,
                    "draft": False,
                    "mergeable": True,
                    "base": {
                        "ref": "main",
                        "sha": BASE,
                        "repo": {"full_name": f"{OWNER}/{REPOSITORY}"},
                    },
                    "head": {"sha": HEAD},
                    "merge_commit_sha": MERGE,
                }
            ).encode()
        if route == f"/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/77/reviews":
            body = json.dumps(
                [
                    {
                        "id": 7001,
                        "user": {"id": self.reviewer_id},
                        "state": "APPROVED",
                        "commit_id": HEAD,
                        "pull_request_url": f"{ORIGIN}/{OWNER}/{REPOSITORY}/pulls/77",
                        "stale": False,
                        "official": True,
                        "dismissed": False,
                    }
                ]
            ).encode()
            return 200, [("X-Total-Count", "1")], body
        return 404, [], b'{"message":"not found"}'


class MissionV3CliProcessTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cargo = shutil.which("cargo")
        curl = shutil.which("curl")
        if cargo is None or curl is None:
            raise unittest.SkipTest("cargo and curl are required for this local process test")

        cls.temporary = tempfile.TemporaryDirectory(prefix="mission-v3-cli-process-", dir="/tmp")
        cls.temp_root = Path(cls.temporary.name)
        cls.package = cls.temp_root / "package"
        cls.package.mkdir(mode=0o700)
        shutil.copy2(ROOT / "Cargo.toml", cls.package / "Cargo.toml")
        shutil.copy2(ROOT / "Cargo.lock", cls.package / "Cargo.lock")
        shutil.copytree(ROOT / "src", cls.package / "src")

        global ORIGIN
        cls.fixture = _Fixture((ROOT / "src" / "lib.rs").read_bytes())
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), cls._handler_for(cls.fixture))
        cls.port = cls.server.server_address[1]
        ORIGIN = f"http://127.0.0.1:{cls.port}"
        source_path = cls.package / "src" / "bin" / "forgejo-mission-local.rs"
        source_text = source_path.read_text(encoding="utf-8")
        fixed_origin = 'const LOCAL_FORGEJO_ORIGIN: &str = "http://127.0.0.1:3130";'
        if source_text.count(fixed_origin) != 1:
            cls.server.server_close()
            raise AssertionError("expected exactly one fixed local profile origin in copied source")
        source_path.write_text(
            source_text.replace(fixed_origin, f'const LOCAL_FORGEJO_ORIGIN: &str = "{ORIGIN}";'),
            encoding="utf-8",
        )
        cls.worker = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.worker.start()

        build_env = {
            "HOME": str(cls.temp_root / "home"),
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "CARGO_HOME": os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
            "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
            "CARGO_TARGET_DIR": str(cls.temp_root / "target"),
        }
        Path(build_env["HOME"]).mkdir(mode=0o700)
        result = subprocess.run(
            [cargo, "build", "--locked", "--offline", "--bin", "forgejo-mission-local"],
            cwd=cls.package,
            env=build_env,
            capture_output=True,
            text=True,
            timeout=600,
            check=False,
        )
        if result.returncode != 0:
            cls.server.shutdown()
            cls.server.server_close()
            cls.worker.join(timeout=5)
            raise AssertionError(
                "offline fixture build failed:\n" + result.stdout[-4000:] + result.stderr[-4000:]
            )
        cls.binary = Path(build_env["CARGO_TARGET_DIR"]) / "debug" / "forgejo-mission-local"
        cls.run_env = {"HOME": build_env["HOME"], "PATH": build_env["PATH"]}

    @classmethod
    def tearDownClass(cls) -> None:
        server = getattr(cls, "server", None)
        if server is not None:
            server.shutdown()
            server.server_close()
        worker = getattr(cls, "worker", None)
        if worker is not None:
            worker.join(timeout=5)
        temporary = getattr(cls, "temporary", None)
        if temporary is not None:
            temporary.cleanup()

    def test_successful_v3_process_and_missing_policy_refusal(self) -> None:
        fixture = self.fixture
        try:
            runtime = self.package / ".runtime"
            policy_dir = runtime / "mission-arbitration"
            policy_dir.mkdir(parents=True, mode=0o700)
            os.chmod(runtime, 0o700)
            os.chmod(policy_dir, 0o700)
            policy = {
                "schema_version": 1,
                "revision": 4,
                "scope": {
                    "origin": ORIGIN,
                    "owner": OWNER,
                    "repository": REPOSITORY,
                    "branch": "main",
                },
                "required_approvals": 1,
                "authorized_reviewer_ids": [9001],
            }
            policy_path = policy_dir / "duration-parser.json"
            policy_path.write_text(json.dumps(policy, separators=(",", ":")) + "\n", encoding="utf-8")
            os.chmod(policy_path, 0o600)
            policy_bytes = policy_path.read_bytes()

            args = [
                str(self.binary),
                "--profile", "duration-parser",
                "--issue", "41",
                "--requirement", "REQ-001",
                "--revision", "1",
                "--allocation-id", "fixture-allocation-001",
                "--workspace-id", "local-demo",
            ]
            accepted = subprocess.run(
                args,
                cwd=self.package,
                env=self.run_env,
                capture_output=True,
                text=True,
                timeout=45,
                check=False,
            )
            self.assertEqual(accepted.returncode, 0, accepted.stdout + accepted.stderr)
            receipt = json.loads(accepted.stdout)
            self.assertEqual(receipt["admission_schema_version"], 3)
            self.assertEqual(receipt["requirement_arbitration"]["reviewer_ids"], [9001])
            self.assertEqual(receipt["requirement_arbitration"]["source_pull_request_number"], 77)
            self.assertEqual(
                receipt["requirement_arbitration"]["requirements_sha256"],
                hashlib.sha256(REQUIREMENTS).hexdigest(),
            )
            self.assertFalse(receipt["agent_runtime_invoked"])
            self.assertTrue((runtime / "mission-execution.jsonl").is_file())
            journal_before = (runtime / "mission-execution.jsonl").read_bytes()
            with fixture.lock:
                accepted_routes = list(fixture.requests)

            policy_path.unlink()
            with fixture.lock:
                fixture.requests.clear()
            refused = subprocess.run(
                args[:-3] + ["fixture-allocation-002", "--workspace-id", "local-demo"],
                cwd=self.package,
                env=self.run_env,
                capture_output=True,
                text=True,
                timeout=45,
                check=False,
            )
            self.assertNotEqual(refused.returncode, 0)
            refusal = json.loads(refused.stdout)
            self.assertEqual(refusal["error"]["code"], "mission_arbitration_policy_missing")
            self.assertEqual((runtime / "mission-execution.jsonl").read_bytes(), journal_before)
            with fixture.lock:
                refused_routes = list(fixture.requests)
            self.assertTrue(refused_routes, "missing-policy run must have been observed by the fixture")
            self.assertFalse(any("/pulls/77" in route for route in refused_routes))

            policy_path.write_bytes(policy_bytes)
            os.chmod(policy_path, 0o600)
            fixture.reviewer_id = 9002
            with fixture.lock:
                fixture.requests.clear()
            unauthorized_args = args.copy()
            unauthorized_args[10] = "fixture-allocation-003"
            unauthorized = subprocess.run(
                unauthorized_args,
                cwd=self.package,
                env=self.run_env,
                capture_output=True,
                text=True,
                timeout=45,
                check=False,
            )
            self.assertNotEqual(unauthorized.returncode, 0)
            unauthorized_refusal = json.loads(unauthorized.stdout)
            self.assertEqual(
                unauthorized_refusal["error"]["code"],
                "mission_requirement_arbitration_insufficient",
            )
            self.assertEqual((runtime / "mission-execution.jsonl").read_bytes(), journal_before)
            with fixture.lock:
                unauthorized_routes = list(fixture.requests)

            self.assertEqual(
                accepted_routes.count(f"/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/77"), 2
            )
            self.assertEqual(
                accepted_routes.count(
                    f"/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/77/reviews?page=1&limit=50"
                ),
                2,
            )
            print(
                "PROCESS_EVIDENCE "
                + json.dumps(
                    {
                        "schema_version": 1,
                        "fixture_only": True,
                        "binary_manifest_root": "temporary copy of Cargo.toml, Cargo.lock, src/",
                        "test_profile_origin": ORIGIN,
                        "temporary_source_change": "fixed profile origin changed only to this test's ephemeral loopback port",
                        "accepted_exit_code": accepted.returncode,
                        "admission_schema_version": receipt["admission_schema_version"],
                        "requirement_sha256": receipt["requirement_arbitration"]["requirements_sha256"],
                        "authorized_fixture_reviewer_ids": receipt["requirement_arbitration"]["reviewer_ids"],
                        "review_snapshot_reads": {
                            "pull_request": accepted_routes.count(
                                f"/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/77"
                            ),
                            "reviews": accepted_routes.count(
                                f"/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/77/reviews?page=1&limit=50"
                            ),
                        },
                        "agent_runtime_invoked": receipt["agent_runtime_invoked"],
                        "missing_policy_error": refusal["error"]["code"],
                        "missing_policy_routes": refused_routes,
                        "journal_unchanged_on_refusal": True,
                        "unauthorized_reviewer_error": unauthorized_refusal["error"]["code"],
                        "journal_unchanged_on_unauthorized_reviewer": True,
                        "unauthorized_reviewer_id": fixture.reviewer_id,
                        "unauthorized_reviewer_routes": unauthorized_routes,
                        "synthetic_ids_are_not_human_identity_evidence": True,
                    },
                    sort_keys=True,
                ),
            )
        finally:
            policy_path = locals().get("policy_path")
            if policy_path is not None:
                policy_path.unlink(missing_ok=True)

    @staticmethod
    def _handler_for(fixture: _Fixture) -> type[BaseHTTPRequestHandler]:
        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def do_GET(self) -> None:  # noqa: N802 - stdlib handler API
                with fixture.lock:
                    fixture.requests.append(self.path)
                status, headers, body = fixture.response(self.path)
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Connection", "close")
                for name, value in headers:
                    self.send_header(name, value)
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, _format: str, *_args: object) -> None:
                return

        return Handler


if __name__ == "__main__":
    unittest.main()
