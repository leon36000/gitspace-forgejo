from pathlib import Path

paths = [
    Path("scripts/run_forgejo_live_webhook_reconcile_emitted.py"),
    Path("tests/test_forgejo_emitted_webhook_e2e.py"),
    Path("docs/refusal-evidence-matrix.md"),
    Path("docs/checkpoint.md"),
    Path("evidence/continuation-433-release-tag-path-segments-native-001/local-run.md"),
    Path("evidence/continuation-433-release-tag-path-segments-native-001/commands.txt"),
    Path(__file__),
]
errors = []
for path in paths:
    data = path.read_bytes()
    if not data.endswith(b"\n"):
        errors.append(f"{path}: missing final newline")
    for number, line in enumerate(data.splitlines(), start=1):
        if line.endswith((b" ", b"\t")):
            errors.append(f"{path}:{number}: trailing whitespace")
if errors:
    raise SystemExit("\n".join(errors))
for path in paths:
    print(f"{path}: clean")
