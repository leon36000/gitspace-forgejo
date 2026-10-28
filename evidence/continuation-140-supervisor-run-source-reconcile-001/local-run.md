# CONTINUATION-140 — reconciliation of supervised run 009 with current source

Date: 2026-10-26. This was a read-only evidence and source-hash audit. It did not rerun Forgejo, an agent, model, verifier, test suite, or merge.

## Finding

The architecture text claimed the trusted launcher had not been implemented and no positive single-writer merge existed. That statement was stale: the repository contains `scripts/run_mission_012_supervised_private_netns_e2e.py`, and the saved [MISSION-012 supervised 009 report](../mission-012-live-agent-webhook-supervised-009/local-run.md) records a local `local-test-adapter` mission which created a Forgejo PR, rejected unauthorized merge attempts, merged through the gate, and verified the exact merge commit after fusion. Its receipts also record the agent's separate PID/network namespace, blocked probes to the configured API, Web and `git-receive-pack` routes, no merge credential in the agent, and a gate capability issued by the harness. The report's policy limit is explicit: the gate does not independently verify that the supervisor excluded every other writer. It was `n=1`, no LLM/AgentOS, no baseline, with the upstream Forgejo source commit unknown.

The current file `scripts/run_mission_012_supervised_private_netns_e2e.py` was inspected statically at `_bwrap_command` and `main`: it configures Bubblewrap user/network isolation, selects `local-test-adapter`, observes the live Forgejo/supervisor namespaces, and reports that supervisor policy is not independently verified. This establishes that a test harness is present today; it is distinct from executing the current hash, because the runner itself is one of the files whose saved run-009 hash differs.

I compared the `composition_source_sha256` saved in `composed-run.json` against the current worktree. The report contains 104 source paths: 76 have identical hashes, 28 differ, and none are missing. The complete old/current hash map and differing path list are in [source-diff.json](source-diff.json). Changes include the supervised runner, composed runner, MCP supervisor, merge CLI, MISSION-012 host, webhook reconciliation and related tests. Thus run 009 is evidence for its saved source snapshot, not validation of today's source. The harness remains a test harness; this audit does not establish a product service or supported production launcher.

The run-009 local append-only manifest was checked read-only after its saved report addendum: 24 artifacts verify, and the prior worktree/project manifest prefixes are preserved. No source file, Forgejo instance, or run-009 artifact was modified.

## Correction and limits

`docs/architecture.md` now distinguishes the existing test harness and its historical successful fixture run from a product launcher, and cites the source drift. `docs/checkpoint.md` records this reconciliation and keeps the project open. The available evidence still does not establish independent writer exclusion, resistance to privileged/root/kernel actors or namespace escape, a current-source end-to-end rerun, an AgentOS/LLM path, or a controlled conventional baseline. No general gain is inferred.

## Reproduction

Run `python3 evidence/continuation-140-supervisor-run-source-reconcile-001/verify_source_snapshot.py` from the product worktree to re-compute the full hash comparison. This script is an evidence verifier, not a runtime or merge authority. The separate command `python3 evidence/mission-012-live-agent-webhook-supervised-009/verify_append_only_indexes.py` checks that prior receipt's manifest without replaying the scenario. The static source query was `rg -n "def _bwrap_command|def main|--unshare-user|--unshare-net|--agent-adapter|supervisor_policy_independently_verified" scripts/run_mission_012_supervised_private_netns_e2e.py`.
