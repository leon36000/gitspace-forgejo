from __future__ import annotations

import hashlib
import hmac
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import run_mission_012_live_admission_e2e as driver  # noqa: E402


class Mission012SignedWebhookTests(unittest.TestCase):
    def test_receiver_command_passes_attestation_seed_only_when_requested(self) -> None:
        base = driver.webhook_receiver_command(
            Path("/bin/receiver"), ("127.0.0.1", 1234),
            Path("/state/secret"), Path("/state/journal"),
        )
        attested = driver.webhook_receiver_command(
            Path("/bin/receiver"), ("127.0.0.1", 1234),
            Path("/state/secret"), Path("/state/journal"),
            Path("/state/receiver-attestation.seed"),
        )

        self.assertEqual(
            base,
            ["/bin/receiver", "127.0.0.1:1234", "/state/secret", "/state/journal"],
        )
        self.assertEqual(
            attested,
            [
                "/bin/receiver", "127.0.0.1:1234", "/state/secret", "/state/journal",
                "/state/receiver-attestation.seed",
            ],
        )

    def test_build_snapshot_excludes_runtime_and_only_copies_build_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "project"
            source.mkdir()
            (source / "src").mkdir()
            (source / "src/main.rs").write_text("fn main() {}\n")
            (source / "Cargo.toml").write_text("[package]\nname='x'\nversion='0.1.0'\nedition='2024'\n")
            (source / "Cargo.lock").write_text("version = 4\n")
            (source / "README.md").write_text("readme is a compile input for include_str!\n")
            cargo = source / ".cargo"
            cargo.mkdir()
            (cargo / "credentials.toml").write_text("token='sentinel-cargo-secret'\n")
            runtime = source / ".runtime"
            runtime.mkdir(mode=0o700)
            (runtime / "forgejo-read.token").write_text("sentinel-secret")
            state = root / "state"
            state.mkdir(mode=0o700)

            snapshot = driver.prepare_build_snapshot(source, state)

            self.assertEqual((snapshot / "Cargo.toml").read_text(), (source / "Cargo.toml").read_text())
            self.assertEqual((snapshot / "src/main.rs").read_text(), "fn main() {}\n")
            self.assertEqual((snapshot / "README.md").read_text(), "readme is a compile input for include_str!\n")
            self.assertFalse((snapshot / ".runtime").exists())
            self.assertFalse((snapshot / ".cargo").exists())
            copied_text = "\n".join(p.read_text() for p in snapshot.rglob("*") if p.is_file())
            self.assertNotIn("sentinel-secret", copied_text)
            self.assertNotIn("sentinel-cargo-secret", copied_text)

    def test_build_snapshot_rejects_symlink_and_hardlinked_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "project"
            source.mkdir()
            (source / "src").mkdir()
            (source / "Cargo.toml").write_text("[package]\nname='x'\nversion='0.1.0'\n")
            (source / "Cargo.lock").write_text("version = 4\n")
            (source / ".runtime").mkdir(mode=0o700)
            secret = source / ".runtime/token"
            secret.write_text("local-only")
            (source / "src/leak.rs").symlink_to(secret)
            state = root / "state"
            state.mkdir(mode=0o700)

            with self.assertRaisesRegex(RuntimeError, "linked or special"):
                driver.prepare_build_snapshot(source, state)

            (source / "src/leak.rs").unlink()
            os.link(secret, source / "src/hardlink.rs")
            other_state = root / "state-hardlink"
            other_state.mkdir(mode=0o700)
            with self.assertRaisesRegex(RuntimeError, "linked or special"):
                driver.prepare_build_snapshot(source, other_state)

    def test_build_snapshot_rejects_source_mutated_after_an_earlier_file_was_copied(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "project"
            source.mkdir()
            (source / "src").mkdir()
            (source / "Cargo.toml").write_text("[package]\nname='old'\nversion='0.1.0'\n")
            (source / "Cargo.lock").write_text("version = 4\n")
            (source / "src/lib.rs").write_text("pub fn value() -> u8 { 1 }\n")
            state = root / "state"
            state.mkdir(mode=0o700)
            original_copy = driver._copy_directory_contents
            changed = False

            def mutate_source_after_directory(*args: object, **kwargs: object) -> None:
                nonlocal changed
                original_copy(*args, **kwargs)
                if not changed:
                    (source / "Cargo.toml").write_text("[package]\nname='new'\nversion='0.1.0'\n")
                    changed = True

            with mock.patch.object(
                driver, "_copy_directory_contents", side_effect=mutate_source_after_directory
            ):
                with self.assertRaisesRegex(RuntimeError, "changed while creating the snapshot"):
                    driver.prepare_build_snapshot(source, state)

    def test_build_snapshot_does_not_write_through_replaced_destination_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "project"
            (source / "src").mkdir(parents=True)
            (source / "Cargo.toml").write_text("[package]\nname='x'\nversion='0.1.0'\n")
            (source / "Cargo.lock").write_text("version = 4\n")
            (source / "src/leak.rs").write_text("pub fn safe() {}\n")
            state = root / "state"
            state.mkdir(mode=0o700)
            outside = root / "outside"
            outside.mkdir(mode=0o700)
            snapshot = state / "build/source-snapshot"
            destination_src = snapshot / "src"
            real_open = os.open
            replaced = False

            def replace_destination_before_open(path: object, flags: int, *args: object, **kwargs: object) -> int:
                nonlocal replaced
                if not replaced and Path(str(path)).name == "leak.rs":
                    dir_fd = kwargs.get("dir_fd")
                    is_destination = (
                        dir_fd is None and Path(str(path)).parent == destination_src
                    )
                    if dir_fd is not None:
                        metadata = os.fstat(int(dir_fd))
                        try:
                            destination_metadata = destination_src.stat()
                        except FileNotFoundError:
                            destination_metadata = None
                        is_destination = destination_metadata is not None and (
                            metadata.st_dev, metadata.st_ino
                        ) == (destination_metadata.st_dev, destination_metadata.st_ino)
                    if is_destination:
                        held = snapshot / "src-held"
                        destination_src.rename(held)
                        destination_src.symlink_to(outside, target_is_directory=True)
                        replaced = True
                return real_open(path, flags, *args, **kwargs)

            with mock.patch.object(driver.os, "open", side_effect=replace_destination_before_open):
                with self.assertRaisesRegex(RuntimeError, "destination|snapshot"):
                    driver.prepare_build_snapshot(source, state)

            self.assertTrue(replaced, "destination replacement hook did not run")
            self.assertFalse((outside / "leak.rs").exists())

    def test_request_signature_covers_exact_body_bytes(self) -> None:
        secret = b"local synthetic receiver secret"
        body = b'{"action":"opened","issue":{"number":1,"state":"open"}}'

        request = driver.signed_webhook_request(secret, "mission012-webhook-1", body)

        headers = request.split(b"\r\n\r\n", 1)[0].split(b"\r\n")
        signature = next(line.split(b": ", 1)[1] for line in headers if line.startswith(b"X-Forgejo-Signature:"))
        self.assertEqual(signature, hmac.new(secret, body, hashlib.sha256).hexdigest().encode())
        self.assertIn(b"Content-Length: " + str(len(body)).encode(), headers)
        self.assertEqual(request.split(b"\r\n\r\n", 1)[1], body)

    def test_request_binds_exact_delivery_id_and_issue_event(self) -> None:
        request = driver.signed_webhook_request(
            b"secret", "mission012-webhook-2", b'{"issue":{"number":1}}'
        )

        self.assertIn(b"X-Forgejo-Delivery: mission012-webhook-2\r\n", request)
        self.assertIn(b"X-Forgejo-Event: issues\r\n", request)
        self.assertTrue(request.startswith(b"POST /forgejo/webhook HTTP/1.1\r\n"))


if __name__ == "__main__":
    unittest.main()
