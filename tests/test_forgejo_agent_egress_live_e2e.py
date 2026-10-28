from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
DRIVER_PATH = ROOT / "scripts" / "run_forgejo_agent_egress_live_e2e.py"


def load_driver():
    if not DRIVER_PATH.is_file():
        raise AssertionError("live Forgejo route egress runner is missing")
    spec = importlib.util.spec_from_file_location("forgejo_agent_egress_live", DRIVER_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError("live Forgejo route egress runner cannot be loaded")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def valid_result() -> dict[str, object]:
    routes = [
        {
            "name": "api",
            "method": "GET",
            "path": "/api/v1/repos/forge-admin/mission-012-clamp/pulls/1",
            "host_status": 404,
            "sandbox_blocked": True,
            "sandbox_status": None,
            "sandbox_errno": 101,
            "sandbox_error_type": "OSError",
            "sandbox_target_port": 3130,
        },
        {
            "name": "web",
            "method": "GET",
            "path": "/forge-admin/mission-012-clamp/pulls/1",
            "host_status": 404,
            "sandbox_blocked": True,
            "sandbox_status": None,
            "sandbox_errno": 101,
            "sandbox_error_type": "OSError",
            "sandbox_target_port": 3130,
        },
        {
            "name": "git_receive_pack",
            "method": "POST",
            "path": "/forge-admin/mission-012-clamp.git/git-receive-pack",
            "host_status": 401,
            "sandbox_blocked": True,
            "sandbox_status": None,
            "sandbox_errno": 101,
            "sandbox_error_type": "OSError",
            "sandbox_target_port": 3130,
        },
    ]
    return {
        "schema_version": 3,
        "verdict": "passed",
        "forgejo": {
            "version": "forgejo version 17.0.0+gitea-1.22.0",
            "binary_sha256": "a" * 64,
            "listener": "127.0.0.1:3130",
        },
        "routes": routes,
        "positive_controls": [
            {"name": "health", "method": "GET", "path": "/api/healthz", "status": 200},
            {"name": "api_repository", "method": "GET", "path": "/api/v1/repos/forge-admin/mission-012-clamp", "status": 200},
            {"name": "web_repository", "method": "GET", "path": "/forge-admin/mission-012-clamp", "status": 200},
        ],
        "trusted_read_control": {
            "status": 200,
            "authorized": True,
            "credential_passed_to_sandbox": False,
        },
        "credential_visible": False,
        "host_systemd_bus_visible": False,
        "network_connected": False,
        "network_namespaces": {"host": "net:[100]", "sandbox": "net:[200]"},
        "branch_sha_before": "b" * 40,
        "branch_sha_after": "b" * 40,
        "sandbox_connect_errors": [
            {"errno": 101, "error_type": "OSError", "port": 3130},
            {"errno": 101, "error_type": "OSError", "port": 3130},
            {"errno": 101, "error_type": "OSError", "port": 3130},
        ],
        "cleanup": {"forgejo_exited": True, "listener_absent": True},
    }


class LiveForgejoEgressEvidenceTests(unittest.TestCase):
    def test_evidence_guard_accepts_and_preserves_its_red_test_log(self) -> None:
        with tempfile.TemporaryDirectory(prefix="forgejo-egress-evidence-") as directory:
            evidence = Path(directory)
            preserved = evidence / "red-route-probe.log"
            preserved.write_text("initial red result\n", encoding="utf-8")

            self.assertIsNone(load_driver().validate_evidence_directory(evidence))
            self.assertEqual(preserved.read_text(encoding="utf-8"), "initial red result\n")

    def test_evidence_guard_rejects_unexpected_existing_artifacts(self) -> None:
        with tempfile.TemporaryDirectory(prefix="forgejo-egress-evidence-") as directory:
            evidence = Path(directory)
            (evidence / "red-route-probe.log").write_text("preserve\n", encoding="utf-8")
            (evidence / "unexpected.json").write_text("{}\n", encoding="utf-8")

            with self.assertRaisesRegex(RuntimeError, "only the preserved red-route-probe.log"):
                load_driver().validate_evidence_directory(evidence)

    def test_accepts_actual_host_routes_and_blocked_sandbox_probes(self) -> None:
        self.assertTrue(load_driver().validate_live_egress_result(valid_result()))

    def test_accepts_legacy_v1_evidence_without_duplicate_error_observations(self) -> None:
        result = valid_result()
        result["schema_version"] = 1
        result.pop("sandbox_connect_errors")
        self.assertTrue(load_driver().validate_live_egress_result(result))

    def test_accepts_legacy_v2_evidence_without_bus_observation(self) -> None:
        result = valid_result()
        result["schema_version"] = 2
        result.pop("host_systemd_bus_visible")
        self.assertTrue(load_driver().validate_live_egress_result(result))

    def test_rejects_a_sandbox_route_that_reached_forgejo(self) -> None:
        result = valid_result()
        result["routes"][0]["sandbox_blocked"] = False  # type: ignore[index]
        result["routes"][0]["sandbox_status"] = 200  # type: ignore[index]
        with self.assertRaisesRegex(ValueError, "sandbox route was not blocked"):
            load_driver().validate_live_egress_result(result)

    def test_rejects_missing_real_host_route_control(self) -> None:
        result = valid_result()
        result["routes"][1]["host_status"] = 503  # type: ignore[index]
        with self.assertRaisesRegex(ValueError, "host route control failed"):
            load_driver().validate_live_egress_result(result)

    def test_rejects_credential_visibility_or_shared_network_namespace(self) -> None:
        for field, value in (
            ("credential_visible", True),
            ("network_connected", True),
            ("network_namespaces", {"host": "net:[100]", "sandbox": "net:[100]"}),
        ):
            with self.subTest(field=field):
                result = valid_result()
                result[field] = value
                with self.assertRaises(ValueError):
                    load_driver().validate_live_egress_result(result)

    def test_rejects_missing_trusted_read_or_changed_branch(self) -> None:
        for mutation in ("trusted_read_control", "branch_sha_after"):
            with self.subTest(mutation=mutation):
                result = valid_result()
                if mutation == "trusted_read_control":
                    result[mutation] = {"status": 401, "authorized": False}
                else:
                    result[mutation] = "c" * 40
                with self.assertRaises(ValueError):
                    load_driver().validate_live_egress_result(result)

    def test_rejects_unconfirmed_forgejo_cleanup(self) -> None:
        result = valid_result()
        result["cleanup"] = {"forgejo_exited": True, "listener_absent": False}
        with self.assertRaises(ValueError):
            load_driver().validate_live_egress_result(result)

    def test_rejects_probe_that_did_not_target_the_real_forgejo_listener(self) -> None:
        result = valid_result()
        result["routes"][2]["sandbox_target_port"] = 51535  # type: ignore[index]
        with self.assertRaisesRegex(ValueError, "did not target the Forgejo listener"):
            load_driver().validate_live_egress_result(result)

    def test_rejects_non_network_errno_as_isolation_evidence(self) -> None:
        result = valid_result()
        result["routes"][0]["sandbox_errno"] = 5  # type: ignore[index]
        with self.assertRaisesRegex(ValueError, "does not confirm network isolation"):
            load_driver().validate_live_egress_result(result)

    def test_rejects_inconsistent_duplicate_route_error_evidence(self) -> None:
        result = valid_result()
        result["sandbox_connect_errors"][0]["errno"] = 111  # type: ignore[index]
        with self.assertRaisesRegex(ValueError, "does not confirm network isolation"):
            load_driver().validate_live_egress_result(result)

    def test_rejects_visible_or_unattested_systemd_bus_in_v3(self) -> None:
        for value in (True, None):
            with self.subTest(value=value):
                result = valid_result()
                result["host_systemd_bus_visible"] = value
                with self.assertRaisesRegex(ValueError, "systemd bus is visible or unverified"):
                    load_driver().validate_live_egress_result(result)

    def test_isolated_probe_process_hides_a_present_host_systemd_bus(self) -> None:
        if not Path("/usr/bin/bwrap").is_file():
            self.skipTest("local Bubblewrap runtime is unavailable")
        host_bus = Path(f"/run/user/{os.getuid()}/bus")
        if not host_bus.is_socket():
            self.skipTest("local user D-Bus socket is unavailable")
        with tempfile.TemporaryDirectory(prefix="forgejo-egress-sandbox-probe-") as directory:
            credential = Path(directory) / "synthetic-credential.txt"
            credential.write_text("SYNTHETIC-CREDENTIAL-CANARY-ONLY\n", encoding="utf-8")
            credential.chmod(0o600)
            observed = load_driver().run_probe_in_sandbox(
                ROOT / "scripts" / "mission_012_test_agent_adapter.py",
                credential,
            )
        self.assertIs(observed["checks"].get("host_systemd_bus_visible"), False)
        self.assertFalse(observed["checks"].get("credential_visible"))
        self.assertFalse(observed["checks"].get("network_connected"))


if __name__ == "__main__":
    unittest.main()
