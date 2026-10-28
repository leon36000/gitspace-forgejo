from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FILES = (
    ROOT / "src/agent_runtime.rs",
    ROOT / "tests/mission_execution_journal_contract.rs",
)

issues = []
for path in FILES:
    for line_number, line in enumerate(path.read_bytes().splitlines(), 1):
        if line.endswith((b" ", b"\t")):
            issues.append(f"{path}:{line_number}: trailing whitespace")

if issues:
    print("\n".join(issues))
    raise SystemExit(1)

print("PASS: no trailing whitespace in untracked Rust source/test files")
