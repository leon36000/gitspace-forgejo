# CONTINUATION-141 — mixed Forgejo/Gitea aliases through reconciliation

Date: 2026-10-26. The run used only local fixtures and the Rust test receiver/CLI. It did not contact a Forgejo server or invoke an LLM/AgentOS.

## Behavior exercised

The new process scenario `actual_reconcile_cli_process_reads_mixed_forgejo_gitea_aliases` uses one signed `issues` delivery with `X-Gitea-Delivery`, `X-Forgejo-Event`, and `X-Gitea-Signature`. This is a single mixed-family pattern, with exactly one name for each logical field. The real webhook receiver accepts it with HTTP 202, writes one canonical `forgejo.webhook.v1.issues` record, and reports sequence 1.

The real `forgejo-reconcile-local` then reads that record and sends exactly one GET to `/api/v1/repos/forge-admin/agent-forge-core/issues/41` on the private loopback HTTP fixture, without an Authorization header. The signed payload says the issue was `closed`; current API fixture state `open` is returned as `current_state_read`. The test compares the inbox bytes before and after reconciliation and confirms they are identical. This demonstrates that the mixed header aliases survive the full receiver→journal→CLI→configured readback path.

The runner executes this port-using test under Bubblewrap with network and PID namespaces. It compiled the locked Rust integration target and local binary dependencies offline, then ran the exact ignored test explicitly: `1 passed; 0 failed; 0 ignored; 117 filtered out`. No full-suite run was performed.

## Verification and metadata

The final `cargo fmt --all -- --check`, runner Python AST syntax check, and `git diff --check` exit 0. The first format check caught one line-wrap difference in the newly added test; `cargo fmt --all` formatted it, and the final isolated scenario then passed. The initial diagnostic is retained as `rustfmt.log`, with the final check as `rustfmt-final.log`.

A fresh offline Cargo metadata guard returns `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, and `spdx_package_license_declared=NOASSERTION`. This keeps Cargo metadata honest without choosing a license or asserting redistribution rights. Current `src`/`tests` search finds only `tests/agent_runtime_contract.rs::FixtureRuntime` implementing `AgentRuntimeV1`; no usable AgentOS/model integration was found or invoked.

## Scope limits

The body, HMAC and API response are synthetic. This is not a live Forgejo/Gitea webhook or reverse proxy test. It covers one mixed alias pattern, not all combinations, header normalization, LLM input, merge authorization, or a full mission. No runtime implementation changed. The previous all-Gitea test CONTINUATION-139 was not rerun. This result does not establish a productivity gain or general security property.

Reproduction command, tool versions, Cargo metadata, test output and source hashes are in this directory.
