from __future__ import annotations

import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(__file__).resolve().parent
COMMAND = [
    str(ROOT / "target/debug/forgejo-reconcile-local"),
    "--merge-local-test",
    "--target-profile",
    "merge-014-agent-forge-core",
    "23",
    "missing-proof.json",
    "missing-expected.json",
    "missing-key",
    "missing-policy.json",
    "missing-review-binding.json",
    "fixture-gate",
    "999999",
]
result = subprocess.run(
    COMMAND,
    check=False,
    capture_output=True,
    text=True,
    env={"PATH": "/usr/bin:/bin", "TMPDIR": "/tmp", "LC_ALL": "C"},
)
try:
    body = json.loads(result.stdout)
except json.JSONDecodeError as error:
    raise SystemExit(f"CLI did not return structured JSON: {error}") from None
if (
    result.returncode != 1
    or body.get("verdict") != "inconclusive"
    or body.get("merge_attempted") is not False
    or body.get("error", {}).get("code")
    != "server_expected_base_precondition_unavailable"
    or result.stderr
):
    raise SystemExit("CLI did not refuse the default multi-writer profile before credential access")
sandbox_command = COMMAND.copy()
sandbox_command[2:2] = ["--writer-admission-profile", "local_single_writer_sandbox"]
sandbox_probe = subprocess.run(
    sandbox_command,
    check=False,
    capture_output=True,
    text=True,
    env={"PATH": "/usr/bin:/bin", "TMPDIR": "/tmp", "LC_ALL": "C"},
)
if (
    sandbox_probe.returncode != 1
    or sandbox_probe.stdout
    or "cannot secure the host credential descriptor" not in sandbox_probe.stderr
):
    raise SystemExit("explicit sandbox profile did not pass admission to credential validation")
(EVIDENCE / "cli-refusal-result.json").write_text(
    json.dumps(
        {
            "argv": COMMAND,
            "returncode": result.returncode,
            "stdout": body,
            "stderr": result.stderr,
            "assertion": "default multiwriter profile refused before credential descriptor read",
            "explicit_sandbox_probe": {
                "argv": sandbox_command,
                "returncode": sandbox_probe.returncode,
                "stderr": sandbox_probe.stderr,
                "assertion": "explicit local sandbox selection passed profile admission and reached invalid-FD validation; it does not prove sandbox enforcement",
            },
        },
        indent=2,
    )
    + "\n"
)
print("PASS: CLI refused the default multi-writer profile before reading the invalid credential FD")
