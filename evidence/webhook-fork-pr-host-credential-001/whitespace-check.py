from pathlib import Path

root = Path(__file__).resolve().parents[2]
paths = [
    root / "tests/webhook_receiver_process_e2e.rs",
    root / "scripts/run_webhook_reconcile_cli_e2e.py",
    root / "docs/superpowers/plans/2026-10-25-fork-pr-host-credential-readback.md",
    root / "docs/checkpoint.md",
    root / "evidence/webhook-fork-pr-host-credential-001/local-run.md",
    root / "evidence/webhook-fork-pr-host-credential-001/commands.txt",
]
errors = []
for path in paths:
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if line.rstrip(" " + chr(9)) != line:
            errors.append(f"{path.relative_to(root)}:{number}: trailing whitespace")
if errors:
    print("\n".join(errors))
    raise SystemExit(1)
print("No trailing whitespace in " + ", ".join(str(path.relative_to(root)) for path in paths))
