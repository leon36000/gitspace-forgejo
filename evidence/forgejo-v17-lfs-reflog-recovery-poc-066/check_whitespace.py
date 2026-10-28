from pathlib import Path

root = Path("/tmp/forgejo-v17-base-cas-poc")
files = [
    root / "services/pull/lfs_recovery.go",
    root / "services/pull/lfs_recovery_test.go",
    root / "tests/integration/git_push_test.go",
]
issues = []
for path in files:
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if line.endswith((" ", "\t")):
            issues.append(f"{path}:{number}: trailing whitespace")
if issues:
    print("\n".join(issues))
    raise SystemExit(1)
print(f"checked {len(files)} Go files: no trailing whitespace")
