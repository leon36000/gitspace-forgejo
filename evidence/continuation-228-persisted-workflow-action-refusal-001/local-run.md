# CONTINUATION-228 — persisted workflow action rejected before API access

Date: 2026-10-27. Scope: one process-level test of the real local receiver and reconciliation CLI. The worktree is dirty at HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; this is not a clean-commit run.

## Scenario and result

`actual_reconcile_cli_process_refuses_persisted_unknown_workflow_action_before_api_read` first submits a valid signed `workflow_run_success` delivery with `action=run_completed` to the real receiver and confirms HTTP 202. The test then changes the persisted action to a future value, recomputes the journal row hash, and confirms that the row still replays as a valid hash chain. It launches the real `forgejo-reconcile-local` CLI with a listener reserved at the configured API endpoint.

The CLI exits unsuccessfully with structured `inconclusive/invalid_envelope`, `details.field_path=payload.action`, `persistence_state=already_persisted`, `source_hmac_persisted=false`, `source_hmac_reverified=false`, and zero upstream reads. The reserved API listener receives no connection, and journal bytes are unchanged. The output action also tells callers not to treat replay as signature re-verification.

The isolated runner was invoked as:

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_persisted_unknown_workflow_action_before_api_read
```

It used `/usr/bin/bwrap --unshare-net --unshare-pid`, copied the current source read-only into the sandbox, compiled offline, and ran the ignored process test there. Exact runner output is in `isolated-cli-process.log`.

## Verification

- Process test: 1 passed, 139 filtered, 0 failed.
- Runner Python syntax parse: passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- Cargo license-state guard: passed; `license=null`, `license_file=null`, `publish=[]`, no license selected, redistribution rights not assessed, SPDX `NOASSERTION`.
- Local evidence manifest: 11/11 entries verified. The append script confirms latest global entries and preserved worktree/project index prefixes.

See adjacent logs, `commands.txt`, `tool-versions.txt`, and `source-sha256.txt` for commands, runtime versions, and file hashes.

## Coverage limits

This proves the local CLI revalidates a hash-chain-valid persisted notification before API I/O. The mutated row is a synthetic fixture, not an event emitted by Forgejo, and recomputing a hash establishes neither authenticity nor correct content. The inbox does not preserve the original HMAC; the CLI reports that limitation. The test does not cover a live Forgejo server, the workflow-job persisted path, LLM/AgentOS, production, or comparative value. Overall mission remains incomplete.
