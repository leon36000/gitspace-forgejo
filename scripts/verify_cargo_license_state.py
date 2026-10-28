#!/usr/bin/env python3
"""Check that Cargo does not claim an unchosen license for this package."""

import json
import subprocess
import sys
from pathlib import Path
from typing import Any


PACKAGE_NAME = "agent-forge-core"
REPOSITORY_ROOT = Path(__file__).resolve().parents[1]


def validate_metadata(metadata: dict[str, Any], package_name: str) -> dict[str, Any]:
    packages = [package for package in metadata.get("packages", []) if package.get("name") == package_name]
    if len(packages) != 1:
        raise ValueError(f"expected exactly one {package_name!r} package in Cargo metadata")

    package = packages[0]
    for field in ("license", "license_file", "publish"):
        if field not in package:
            raise ValueError(f"Cargo metadata omitted required package field {field!r}")
    if package.get("license") is not None:
        raise ValueError("license must remain unset until the rights holder chooses one")
    if package.get("license_file") is not None:
        raise ValueError("license_file must remain unset until the rights holder chooses one")
    if package.get("publish") != []:
        raise ValueError("publishing must remain disabled while licensing is undecided")

    package_metadata = package.get("metadata")
    agent_forge_metadata = (
        package_metadata.get("agent-forge")
        if isinstance(package_metadata, dict)
        else None
    )
    licensing_review = (
        agent_forge_metadata.get("licensing")
        if isinstance(agent_forge_metadata, dict)
        else None
    )
    if (
        not isinstance(licensing_review, dict)
        or licensing_review.get("decision") != "no_license_selected"
    ):
        raise ValueError("review metadata must declare no_license_selected until the rights holder decides")
    redistribution_rights = licensing_review.get("redistribution_rights")
    if redistribution_rights != "not_assessed":
        raise ValueError("redistribution rights must remain not_assessed")
    spdx_package_license_declared = licensing_review.get("spdx_package_license_declared")
    if spdx_package_license_declared != "NOASSERTION":
        raise ValueError("SPDX export marker must remain NOASSERTION until the rights holder decides")

    return {
        "status": "metadata_policy_verified",
        "package": package_name,
        "license_decision": licensing_review["decision"],
        "license": None,
        "license_file": None,
        "publish": [],
        "redistribution_rights": redistribution_rights,
        "spdx_package_license_declared": spdx_package_license_declared,
    }


def main() -> int:
    command = [
        "cargo",
        "metadata",
        "--locked",
        "--offline",
        "--format-version",
        "1",
        "--no-deps",
    ]
    result = subprocess.run(
        command,
        cwd=REPOSITORY_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        return result.returncode

    try:
        report = validate_metadata(json.loads(result.stdout), PACKAGE_NAME)
    except (json.JSONDecodeError, ValueError) as error:
        print(json.dumps({"status": "rejected", "error": str(error)}, sort_keys=True))
        return 1

    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
