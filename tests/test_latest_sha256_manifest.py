from __future__ import annotations

import hashlib
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from verify_latest_sha256_manifest import verify_latest_manifest  # noqa: E402


class LatestSha256ManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="latest-sha256-manifest-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.manifest = self.root / "SHA256SUMS"

    def write_file(self, name: str, data: bytes) -> str:
        path = self.root / name
        path.write_bytes(data)
        return hashlib.sha256(data).hexdigest()

    def test_latest_valid_hash_supersedes_older_hash_for_same_path(self) -> None:
        digest = self.write_file("artifact.txt", b"current")
        self.manifest.write_text(
            f"{'0' * 64}  artifact.txt\n{digest}  artifact.txt\n",
            encoding="utf-8",
        )

        result = verify_latest_manifest(self.manifest, self.root)

        self.assertTrue(result["passed"])
        self.assertEqual(result["valid_records"], 2)
        self.assertEqual(result["unique_paths"], 1)
        self.assertEqual(result["verified_paths"], 1)

    def test_detects_a_current_file_mismatch(self) -> None:
        self.write_file("artifact.txt", b"current")
        self.manifest.write_text(f"{'0' * 64}  artifact.txt\n", encoding="utf-8")

        result = verify_latest_manifest(self.manifest, self.root)

        self.assertFalse(result["passed"])
        self.assertEqual(result["mismatched_path_count"], 1)

    def test_legacy_none_record_requires_a_later_valid_digest(self) -> None:
        digest = self.write_file("artifact.txt", b"current")
        self.manifest.write_text(f"None  artifact.txt\n{digest}  artifact.txt\n", encoding="utf-8")

        result = verify_latest_manifest(self.manifest, self.root)

        self.assertTrue(result["passed"])
        self.assertEqual(result["legacy_unhashed_records"], 1)

    def test_missing_and_escaping_paths_fail_closed(self) -> None:
        digest = "0" * 64
        self.manifest.write_text(
            f"{digest}  missing.txt\n{digest}  ../outside.txt\n",
            encoding="utf-8",
        )

        result = verify_latest_manifest(self.manifest, self.root)

        self.assertFalse(result["passed"])
        self.assertEqual(result["missing_path_count"], 1)
        self.assertEqual(result["unsafe_path_count"], 1)

    def test_blank_lines_are_ignored_and_other_malformed_records_fail(self) -> None:
        digest = self.write_file("artifact.txt", b"current")
        self.manifest.write_text(f"\n{digest}  artifact.txt\ninvalid\n", encoding="utf-8")

        result = verify_latest_manifest(self.manifest, self.root)

        self.assertFalse(result["passed"])
        self.assertEqual(result["blank_lines_ignored"], 1)
        self.assertEqual(len(result["malformed_records"]), 1)


if __name__ == "__main__":
    unittest.main()
