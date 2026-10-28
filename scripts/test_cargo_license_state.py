import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from verify_cargo_license_state import validate_metadata


class CargoLicenseStateTests(unittest.TestCase):
    def setUp(self):
        self.metadata = {
            "packages": [
                {
                    "name": "agent-forge-core",
                    "license": None,
                    "license_file": None,
                    "publish": [],
                    "metadata": {
                        "agent-forge": {
                            "licensing": {
                                "decision": "no_license_selected",
                                "redistribution_rights": "not_assessed",
                            "spdx_package_license_declared": "NOASSERTION",
                            }
                        }
                    },
                }
            ]
        }

    def test_accepts_explicit_no_license_selection_with_publication_disabled(self):
        result = validate_metadata(self.metadata, "agent-forge-core")
        self.assertEqual(result["status"], "metadata_policy_verified")
        self.assertEqual(result["license_decision"], "no_license_selected")
        self.assertIsNone(result["license"])
        self.assertIsNone(result["license_file"])
        self.assertEqual(result["publish"], [])
        self.assertEqual(result["redistribution_rights"], "not_assessed")
        self.assertEqual(result["spdx_package_license_declared"], "NOASSERTION")

    def test_rejects_spdx_export_marker_that_asserts_a_license(self):
        review = self.metadata["packages"][0]["metadata"]["agent-forge"]["licensing"]
        review["spdx_package_license_declared"] = "MIT"
        with self.assertRaisesRegex(ValueError, "SPDX export marker must remain NOASSERTION"):
            validate_metadata(self.metadata, "agent-forge-core")

    def test_requires_spdx_export_marker(self):
        review = self.metadata["packages"][0]["metadata"]["agent-forge"]["licensing"]
        del review["spdx_package_license_declared"]
        with self.assertRaisesRegex(ValueError, "SPDX export marker must remain NOASSERTION"):
            validate_metadata(self.metadata, "agent-forge-core")

    def test_requires_explicit_no_license_selection_review_metadata(self):
        del self.metadata["packages"][0]["metadata"]
        with self.assertRaisesRegex(ValueError, "review metadata must declare no_license_selected"):
            validate_metadata(self.metadata, "agent-forge-core")

    def test_rejects_metadata_that_claims_a_license_choice_or_assessed_rights(self):
        review = self.metadata["packages"][0]["metadata"]["agent-forge"]["licensing"]
        review["decision"] = "selected"
        with self.assertRaisesRegex(ValueError, "review metadata must declare no_license_selected"):
            validate_metadata(self.metadata, "agent-forge-core")

        review["decision"] = "no_license_selected"
        review["redistribution_rights"] = "approved"
        with self.assertRaisesRegex(ValueError, "redistribution rights must remain not_assessed"):
            validate_metadata(self.metadata, "agent-forge-core")

    def test_rejects_license_identifier(self):
        self.metadata["packages"][0]["license"] = "MIT"
        with self.assertRaisesRegex(ValueError, "license must remain unset"):
            validate_metadata(self.metadata, "agent-forge-core")

    def test_rejects_omitted_license_field(self):
        del self.metadata["packages"][0]["license"]
        with self.assertRaisesRegex(ValueError, "Cargo metadata omitted required package field 'license'"):
            validate_metadata(self.metadata, "agent-forge-core")

    def test_rejects_license_file(self):
        self.metadata["packages"][0]["license_file"] = "LICENSE"
        with self.assertRaisesRegex(ValueError, "license_file must remain unset"):
            validate_metadata(self.metadata, "agent-forge-core")

    def test_rejects_omitted_license_file_field(self):
        del self.metadata["packages"][0]["license_file"]
        with self.assertRaisesRegex(ValueError, "Cargo metadata omitted required package field 'license_file'"):
            validate_metadata(self.metadata, "agent-forge-core")

    def test_rejects_enabled_publication(self):
        self.metadata["packages"][0]["publish"] = ["crates-io"]
        with self.assertRaisesRegex(ValueError, "publishing must remain disabled"):
            validate_metadata(self.metadata, "agent-forge-core")

    def test_rejects_omitted_publish_field(self):
        del self.metadata["packages"][0]["publish"]
        with self.assertRaisesRegex(ValueError, "Cargo metadata omitted required package field 'publish'"):
            validate_metadata(self.metadata, "agent-forge-core")

    def test_rejects_missing_or_ambiguous_package(self):
        with self.assertRaisesRegex(ValueError, "expected exactly one .* package"):
            validate_metadata({"packages": []}, "agent-forge-core")
        self.metadata["packages"].append(dict(self.metadata["packages"][0]))
        with self.assertRaisesRegex(ValueError, "expected exactly one .* package"):
            validate_metadata(self.metadata, "agent-forge-core")


if __name__ == "__main__":
    unittest.main()
