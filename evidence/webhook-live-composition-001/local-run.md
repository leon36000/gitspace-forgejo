# Local composition verification

- Recorded: `2026-10-24T05:40:36Z`.
- Source worktree HEAD: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; the worktree is dirty and relevant source hashes are listed in `source-hashes.txt`.
- Scope: standalone PR preflight cost validation and composed-runner stage ordering, plus the five Python contract suites.
- Environment: local Python 3.12. No Forgejo service or model call was launched in this verification.

## Changes verified

The standalone PR driver now requires `--budget-cap-usd`, validates that the configured cap is positive and no higher than USD 0.20, checks that the agent-reported cap is valid and no higher than the configured cap, and refuses a missing, malformed, boolean, non-finite, negative, or over-cap reported cost before creating evidence or starting Forgejo. The composed driver passes the configured cap to both the agent and PR stages and compares reported cost to the stricter agent-reported cap before it starts the PR stage.

## Suites

- `tests/test_forgejo_live_webhook_mission_admission.py`: 12 tests, exit 0
- `tests/test_mission_012_live_composed_e2e.py`: 22 tests, exit 0
- `tests/test_mission_012_live_agent_pr_e2e.py`: 11 tests, exit 0
- `tests/test_mission_012_live_admission_e2e.py`: 6 tests, exit 0
- `tests/test_mission_012_mcp_supervisor.py`: 10 tests, exit 0

Total: 61 tests, 0 failures. Full command output is retained in the matching `test_*.log` files.

## Other checks

- `python3 -m py_compile` for both drivers and both changed test files: exit 0.
- `python3 scripts/run_mission_012_live_agent_pr_e2e.py --help`: exit 0 and lists the required budget flag.
- `git diff --check`: exit 0.
- Targeted whitespace and final-newline check for both drivers and tests: passed.
- `sha256sum -c SHA256SUMS`: run after this report and source hash list were written.

## Failure-first evidence

The new test `test_stage_stops_before_pr_when_cost_exceeds_agent_reported_cap` first failed with `AssertionError: RuntimeError not raised`; the composite accepted a cost of USD 0.15 under a reported USD 0.10 cap and proceeded to PR. After changing the comparison to the effective reported cap, the same test passed and confirmed only the admission and agent stages ran. The direct PR tests confirm rejected costs do not call `subprocess.Popen` and do not create evidence.

## Independent review

A read-only reviewer first identified that the composite compared cost to the configured cap instead of the stricter reported cap. After the change, the reviewer confirmed the mismatch is closed, all invalid/missing/over-cap cost cases are checked before Forgejo starts in the standalone PR driver, and the composed path stops before PR. The reviewer ran no test, service, or model and changed no files. No paid Claude Sonnet subagent was started for this tranche.

## Cost and execution limits

The model cost remains a value reported by the local Claude CLI, not independently attested billing data. The current [Claude Code CLI reference](https://code.claude.com/docs/en/cli-usage) describes `--max-budget-usd` as using a client-side estimate that may differ from the bill and explicitly says spend can pass the cap. An earlier invocation configured with `--max-budget-usd 0.20` reported USD `0.5258432` and `error_max_budget_usd`; this option has not demonstrated a hard spend ceiling. No new model call was launched. At `2026-10-24T05:40:36Z`, PID `1947069` remained present, started at `2026-10-23 20:58:59` local; the Ralph marker mtime remained `2026-10-23 21:00:03` local. Only PID/state/start time and marker metadata were checked; no command line, prompt, environment, logs, transcript, or terminal were read.

These contract tests do not demonstrate a fresh composed run with a Forgejo webhook, LLM call, PR, independent verification, decision, and sandbox merge. They establish the local gates and ordering only.
