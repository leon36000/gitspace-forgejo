# MISSION-012 supervised private Forgejo run 009

Date: 2026-10-26. All services and test data were local. No production service, external model API, secret service, publication, or new access was used.

## Purpose

CONTINUATION-110 left the latest supervised Forgejo PR run with a Forgejo readback after merge but no Cargo acceptance execution against the exact merge commit. This run retains the isolated local-test-adapter path and adds that check: the runner fetches and materializes the merge commit reported by Forgejo, checks its complete fixture contents against the independently verified PR candidate and protected acceptance file, then executes the acceptance suite from that materialized merge snapshot.

## Reproduction

From the product worktree:

```text
python3 scripts/run_mission_012_supervised_private_netns_e2e.py --state-dir /tmp/forgejo-supervised-netns-20261026-009 --forgejo-binary /tmp/forgejo-17.0.0 --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/mission-012-live-agent-webhook-supervised-009 --execution-id live-mcp-029 --allocation-id mission012-supervised-009
```

Exit code: 0. The outer observation has verdict `passed` and 14/14 named checks true, including the post-merge test binding. Forgejo's local binary reported `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; its upstream source commit is not established. Bubblewrap was 0.9.0.

## Observed path and result

- A versioned MISSION-012 clamp requirement and protected acceptance file produced a real base failure: `values_outside_the_range_clamp_to_the_nearest_boundary` observed `-2` instead of `10` (`1 passed; 1 failed`).
- The deterministic `local-test-adapter` used the supplied context, tools and writable source scope, changed only `src/lib.rs`, and passed the independent candidate and pre-merge tests (`2 passed; 0 failed`). This is a harness adapter, not a language model or AgentOS.
- Forgejo created private PR #2. A merge without the required review and a merge by a non-whitelisted reviewer each received HTTP 405. The allowed independent-review/gate path later merged with HTTP 200.
- Forgejo readback reports base `5d18049b0552d0a064796e2b7cd568377697e48e`, candidate `72475930228c7a0230b15dda9fedb1522ce4089d`, and merge `0d9780667958d4c3d58f665e54aa99ffc5f165ea`. `main` points to the merge commit; PR #2 and issue #1 are closed. The merge tree matches the candidate tree and only `src/lib.rs` differs from the base.
- The post-merge verifier checks out the exact merge SHA from the trusted local Git mirror, compares every fixture file to the verified candidate, confirms the acceptance hash, and runs `cargo test --offline --locked --test acceptance` with a fresh target/home, read-only merged snapshot and toolchain, no network, no gate credential, and a bounded systemd/Bubblewrap sandbox. Exit is 0; 2/2 tests pass. The receipt records the merge, base, candidate, requirements/policy, source and test hashes, Git tree OID, command, environment, duration and output hash.
- The agent subprocess had a distinct network/PID namespace, no visible credential, and only FDs 0–2. Its direct probes to the configured Forgejo API, Web and `git-receive-pack` routes were blocked; the host supervisor observed zero route connections. The gate, not the agent, received the merge credential. The supervisor and Forgejo shared a rootless private network namespace; a host loopback connection was refused and unprivileged namespace joining was denied.

## Verification and limits

`tests/test_mission_012_live_agent_pr_e2e.py` passes 46/46. Python `py_compile` and `git diff --check` exit 0; see `continuation-111-focused-checks.log`. Cargo product tests were not rerun in this lot because the product Rust sources did not change; the post-merge Cargo acceptance run is recorded separately.

After run 009, the strengthened supervisor predicate was applied read-only to its saved `forgejo-result.json`, `postmerge-verification.json` and `postmerge-cargo-test.log`. It recomputed the log byte count and SHA-256, required the exact two-test Cargo success line, and checked all commit/source/criteria/tree bindings; the result is `pass` in `postmerge-artifact-binding-validation.json`. Neither Forgejo nor Cargo was rerun for this check. Run 009's original 14-check outer receipt predates that extra artifact cross-check; future live invocations now perform it in the outer runner.

The local-run report received this final addendum before the manifests were closed. The local `SHA256SUMS` keeps the earlier report digest as an append-only revision and records the final digest on a later line; the verifier checks the latest digest for each path and the saved manifest prefixes.

This is one local deterministic run (`n=1`), with no LLM, AgentOS, real model tokens, or comparative baseline; reported model cost is zero. The agent route check covers the three pinned routes, not arbitrary routes or privileged access. The gate binds and rechecks live process identities, but writer exclusion still rests on a trusted supervisor policy that the gate cannot independently verify. Root, privileged host actors, kernel compromise and namespace escape are outside the test model. The Forgejo binary hash is pinned, but its exact upstream commit is unknown. No general success gain is inferred.

The receipts, source snapshot, command outputs and checksums in this directory are local evidence. No Forgejo or GitSpace source code was copied into the product.
