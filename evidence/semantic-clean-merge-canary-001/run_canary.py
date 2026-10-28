#!/usr/bin/env python3
"""Reproduce a clean Git merge whose combined behavior needs protected review."""

from __future__ import annotations

import hashlib
import json
import argparse
from pathlib import Path
import subprocess
import tempfile


EVIDENCE = Path(__file__).resolve().parent
LIB_BASE = """#[derive(Debug, PartialEq, Eq)]
pub enum BoundsError {
    EmptyRange,
}

fn lower_bound(min: i32, max: i32, exclusive: bool) -> i32 {
    let _range_width = max.saturating_sub(min);
    let _is_exclusive = exclusive;
    min
}

fn upper_bound(min: i32, max: i32, exclusive: bool) -> i32 {
    let _range_width = max.saturating_sub(min);
    let _is_exclusive = exclusive;
    max
}

pub fn clamp(
    value: i32,
    min: i32,
    max: i32,
    exclusive_min: bool,
    exclusive_max: bool,
) -> Result<i32, BoundsError> {
    if min > max {
        return Err(BoundsError::EmptyRange);
    }
    let effective_min = lower_bound(min, max, exclusive_min);
    let effective_max = upper_bound(min, max, exclusive_max);
    Ok(value.clamp(effective_min, effective_max))
}
"""
LOWER_OLD = "    let _is_exclusive = exclusive;\n    min"
LOWER_CHANGE = "    let _is_exclusive = exclusive;\n    if exclusive && min < max { min + 1 } else { min }"
UPPER_OLD = "    let _is_exclusive = exclusive;\n    max"
UPPER_CHANGE = "    let _is_exclusive = exclusive;\n    if exclusive && min < max { max - 1 } else { max }"
LOWER_TEST = """use semantic_merge_canary::clamp;

#[test]
fn exclusive_lower_bound_keeps_the_upper_bound_closed() {
    assert_eq!(clamp(0, 0, 1, true, false), Ok(1));
}
"""
UPPER_TEST = """use semantic_merge_canary::clamp;

#[test]
fn exclusive_upper_bound_keeps_the_lower_bound_closed() {
    assert_eq!(clamp(1, 0, 1, false, true), Ok(0));
}
"""
PROTECTED_TEST = """use semantic_merge_canary::{clamp, BoundsError};

#[test]
fn two_exclusive_bounds_on_adjacent_values_are_reported_as_empty() {
    assert_eq!(clamp(0, 0, 1, true, true), Err(BoundsError::EmptyRange));
}
"""


RUN_LABEL = "attempt-002"


def run(name: str, command: list[str], cwd: Path, expected: int = 0) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, check=False)
    (EVIDENCE / f"{RUN_LABEL}-{name}").write_text(
        f"$ {' '.join(command)}\nexit={result.returncode}\n--- stdout ---\n"
        f"{result.stdout}\n--- stderr ---\n{result.stderr}",
        encoding="utf-8",
    )
    if result.returncode != expected:
        raise RuntimeError(f"{name}: expected exit {expected}, got {result.returncode}")
    return result


def write(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value, encoding="utf-8")


def main() -> None:
    global RUN_LABEL
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-label", default="attempt-002")
    args = parser.parse_args()
    if not args.run_label.replace("-", "").isalnum():
        raise SystemExit("run label must contain only letters, digits, and hyphens")
    RUN_LABEL = args.run_label
    summary: dict[str, object] = {
        "result": "running",
        "scope": "isolated temporary Rust/Git fixture; no Forgejo or product gate",
    }
    with tempfile.TemporaryDirectory(prefix="semantic-clean-merge-") as tmp:
        repo = Path(tmp)
        write(
            repo / "Cargo.toml",
            '[package]\nname = "semantic-merge-canary"\nversion = "0.1.0"\nedition = "2024"\n',
        )
        write(repo / "src/lib.rs", LIB_BASE)
        write(repo / "tests/placeholder.rs", "#[test]\nfn fixture_compiles() {}\n")
        run("cargo-lock.log", ["cargo", "generate-lockfile", "--offline"], repo)
        run("git-init.log", ["git", "init", "--initial-branch=main"], repo)
        run("git-config-name.log", ["git", "config", "user.name", "Canary"], repo)
        run("git-config-email.log", ["git", "config", "user.email", "canary@localhost"], repo)
        run("git-add-base.log", ["git", "add", "Cargo.toml", "Cargo.lock", "src", "tests"], repo)
        run("git-commit-base.log", ["git", "commit", "-m", "base"], repo)
        base_sha = run("git-base-sha.log", ["git", "rev-parse", "HEAD"], repo).stdout.strip()

        run("git-branch-lower.log", ["git", "switch", "-c", "exclusive-lower"], repo)
        source = (repo / "src/lib.rs").read_text(encoding="utf-8")
        if source.count(LOWER_OLD) != 1:
            raise RuntimeError("lower-bound edit anchor is not unique")
        write(repo / "src/lib.rs", source.replace(LOWER_OLD, LOWER_CHANGE))
        write(repo / "tests/exclusive_lower.rs", LOWER_TEST)
        run("git-add-lower.log", ["git", "add", "src/lib.rs", "tests/exclusive_lower.rs"], repo)
        run("git-commit-lower.log", ["git", "commit", "-m", "make lower bound exclusive"], repo)
        lower_sha = run("git-lower-sha.log", ["git", "rev-parse", "HEAD"], repo).stdout.strip()
        run("lower-branch-test.log", ["cargo", "test", "--offline", "--locked", "--test", "exclusive_lower"], repo)

        run("git-switch-main.log", ["git", "switch", "main"], repo)
        run("git-branch-upper.log", ["git", "switch", "-c", "exclusive-upper"], repo)
        source = (repo / "src/lib.rs").read_text(encoding="utf-8")
        if source.count(UPPER_OLD) != 1:
            raise RuntimeError("upper-bound edit anchor is not unique")
        write(repo / "src/lib.rs", source.replace(UPPER_OLD, UPPER_CHANGE))
        write(repo / "tests/exclusive_upper.rs", UPPER_TEST)
        run("git-add-upper.log", ["git", "add", "src/lib.rs", "tests/exclusive_upper.rs"], repo)
        run("git-commit-upper.log", ["git", "commit", "-m", "make upper bound exclusive"], repo)
        upper_sha = run("git-upper-sha.log", ["git", "rev-parse", "HEAD"], repo).stdout.strip()
        run("upper-branch-test.log", ["cargo", "test", "--offline", "--locked", "--test", "exclusive_upper"], repo)

        tree = run(
            "merge-tree.log",
            ["git", "merge-tree", "--write-tree", "--name-only", "HEAD", "exclusive-lower"],
            repo,
        )
        merge_tree = tree.stdout.splitlines()[0].strip()
        run("git-merge.log", ["git", "merge", "--no-edit", "exclusive-lower"], repo)
        combined_sha = run("git-combined-sha.log", ["git", "rev-parse", "HEAD"], repo).stdout.strip()

        # Both branch-specific checks remain green after the clean merge.
        run("combined-lower-test.log", ["cargo", "test", "--offline", "--locked", "--test", "exclusive_lower"], repo)
        run("combined-upper-test.log", ["cargo", "test", "--offline", "--locked", "--test", "exclusive_upper"], repo)
        write(repo / "tests/protected_interaction.rs", PROTECTED_TEST)
        run("git-add-protected-test.log", ["git", "add", "tests/protected_interaction.rs"], repo)
        failing = run(
            "protected-red.log",
            ["cargo", "test", "--offline", "--locked", "--test", "protected_interaction"],
            repo,
            expected=101,
        )
        failure_actionable = "min > max" in (failing.stdout + failing.stderr)
        if not failure_actionable:
            raise RuntimeError("protected failure did not expose the inverted effective interval")

        # Integration correction: reject an empty effective interval before calling i32::clamp.
        source = (repo / "src/lib.rs").read_text(encoding="utf-8")
        anchor = "    let effective_max = upper_bound(min, max, exclusive_max);\n    Ok(value.clamp(effective_min, effective_max))"
        corrected = (
            "    let effective_max = upper_bound(min, max, exclusive_max);\n"
            "    if effective_min > effective_max {\n"
            "        return Err(BoundsError::EmptyRange);\n"
            "    }\n    Ok(value.clamp(effective_min, effective_max))"
        )
        if source.count(anchor) != 1:
            raise RuntimeError("correction anchor is not unique")
        write(repo / "src/lib.rs", source.replace(anchor, corrected))
        run("corrected-protected-test.log", ["cargo", "test", "--offline", "--locked", "--test", "protected_interaction"], repo)
        run("corrected-combined-suite.log", ["cargo", "test", "--offline", "--locked"], repo)
        write(EVIDENCE / f"{RUN_LABEL}-combined-src-lib.rs", (repo / "src/lib.rs").read_text(encoding="utf-8"))
        write(EVIDENCE / f"{RUN_LABEL}-protected-interaction.rs", PROTECTED_TEST)
        summary.update(
            {
                "result": "pass",
                "base_commit": base_sha,
                "lower_change_commit": lower_sha,
                "upper_change_commit": upper_sha,
                "merge_tree": merge_tree,
                "combined_commit": combined_sha,
                "textual_merge": "clean (git merge-tree exit 0; no conflicted paths)",
                "independent_branch_tests": "both pass",
                "combined_branch_tests": "both pass before and after correction",
                "protected_combined_acceptance": "fails before correction with inverted effective interval; passes after explicit EmptyRange guard",
                "actionable_failure_observed": failure_actionable,
                "correction": "check effective_min > effective_max and return EmptyRange before i32::clamp",
                "limits": ["fixture only", "no Forgejo PR or production merge gate", "no LLM or AgentOS"],
            }
        )
    (EVIDENCE / f"{RUN_LABEL}-result-summary.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    hashes = {}
    for path in sorted(EVIDENCE.iterdir()):
        if path.is_file() and path.name not in {"source-hashes.json", "SHA256SUMS"}:
            hashes[path.name] = hashlib.sha256(path.read_bytes()).hexdigest()
    (EVIDENCE / f"{RUN_LABEL}-source-hashes.json").write_text(
        json.dumps(hashes, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(summary, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
