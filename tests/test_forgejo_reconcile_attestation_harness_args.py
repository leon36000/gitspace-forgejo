from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import run_forgejo_live_webhook_reconcile_e2e as reconcile_harness  # noqa: E402
import run_forgejo_live_webhook_reconcile_emitted as emitted_harness  # noqa: E402

PUBLIC_KEY = "a" * 64


class ReconcileAttestationHarnessArgsTests(unittest.TestCase):
    def test_receiver_public_key_derivation_uses_only_seed_path_and_returns_canonical_hex(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[], returncode=0, stdout=(PUBLIC_KEY + "\n").encode(), stderr=b""
        )
        with patch.object(reconcile_harness.subprocess, "run", return_value=completed) as run:
            key = reconcile_harness.derive_receiver_attestation_public_key(
                Path("/test/forgejo-webhook-local"),
                Path("/tmp/test-state"),
                Path("/tmp/test-state/receiver-attestation.seed"),
            )
        self.assertEqual(key, PUBLIC_KEY)
        self.assertEqual(
            run.call_args.args[0],
            [
                "/test/forgejo-webhook-local",
                "--print-attestation-public-key",
                "/tmp/test-state/receiver-attestation.seed",
            ],
        )

    def test_single_reconciliation_passes_public_key_without_putting_token_in_argv(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[], returncode=1, stdout=b'{"verdict":"inconclusive"}', stderr=b""
        )
        token = "synthetic-token-never-in-argv"
        with patch.object(reconcile_harness.subprocess, "run", return_value=completed) as run:
            status, result = reconcile_harness.run_reconcile(
                Path("/test/forgejo-reconcile-local"),
                Path("/tmp/test-state"),
                Path("/tmp/test-state/inbox.jsonl"),
                "delivery-1",
                token,
                trusted_attestation_public_key=PUBLIC_KEY,
            )
        argv = run.call_args.args[0]
        self.assertEqual(status, 1)
        self.assertEqual(result, {"verdict": "inconclusive"})
        self.assertEqual(
            argv[1:3],
            ["--trusted-webhook-attestation-public-key-hex", PUBLIC_KEY],
        )
        self.assertIn("--host-credential-fd", argv)
        self.assertNotIn(token, argv)
        fd_index = argv.index("--host-credential-fd") + 1
        self.assertEqual(run.call_args.kwargs["pass_fds"], (int(argv[fd_index]),))

    def test_single_reconciliation_reads_public_policy_from_owner_only_run_state(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[], returncode=1, stdout=b'{"verdict":"inconclusive"}', stderr=b""
        )
        with tempfile.TemporaryDirectory(prefix="reconcile-attestation-harness-") as temp:
            state = Path(temp)
            public_key_path = state / "receiver-attestation.public"
            public_key_path.write_text(PUBLIC_KEY + "\n", encoding="ascii")
            public_key_path.chmod(0o600)
            with patch.object(reconcile_harness.subprocess, "run", return_value=completed) as run:
                reconcile_harness.run_reconcile(
                    Path("/test/forgejo-reconcile-local"),
                    state,
                    state / "inbox.jsonl",
                    "delivery-1",
                    "synthetic-token",
                )
        argv = run.call_args.args[0]
        self.assertEqual(argv[1:3], ["--trusted-webhook-attestation-public-key-hex", PUBLIC_KEY])

    def test_batch_reconciliation_passes_public_key_and_keeps_token_in_pipe(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[], returncode=1, stdout=b'{"verdict":"inconclusive"}', stderr=b""
        )
        token = "synthetic-batch-token-never-in-argv"
        with patch.object(emitted_harness.subprocess, "run", return_value=completed) as run:
            status, result, remaining = emitted_harness.run_reconcile_batch(
                Path("/test/forgejo-reconcile-local"),
                Path("/tmp/test-state"),
                Path("/tmp/test-state/inbox.jsonl"),
                ["delivery-1", "delivery-2"],
                token,
                PUBLIC_KEY,
            )
        argv = run.call_args.args[0]
        self.assertEqual(status, 1)
        self.assertEqual(result, {"verdict": "inconclusive"})
        self.assertEqual(
            argv[1:3],
            ["--trusted-webhook-attestation-public-key-hex", PUBLIC_KEY],
        )
        self.assertIn("--host-credential-fd", argv)
        self.assertNotIn(token, argv)
        self.assertEqual(remaining, token.encode())

    def test_noncanonical_public_keys_are_rejected_before_spawning_cli(self) -> None:
        with patch.object(reconcile_harness.subprocess, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "canonical lower-case"):
                reconcile_harness.run_reconcile(
                    Path("/test/forgejo-reconcile-local"),
                    Path("/tmp/test-state"),
                    Path("/tmp/test-state/inbox.jsonl"),
                    "delivery-1",
                    "synthetic-token",
                    trusted_attestation_public_key=PUBLIC_KEY.upper(),
                )
        run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
