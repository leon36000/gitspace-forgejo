# FORGEJO-NATIVE-PUSH-NETWORK-ISOLATION-006

Date: 2026-10-25. Follow-up against the durable journal from [FORGEJO-NATIVE-PUSH-PARTIAL-SELECTION-005](../forgejo-native-push-webhook-reconcile-005/local-run.md); no Forgejo push was repeated.

## Question and result

Does the reconciler still refuse an absent ID when its process cannot reach host loopback, and does it preserve the credential pipe and journal?

The helper selected one real push delivery emitted by Forgejo (`b0ef43d2-e5f9-442d-bac9-285658f659b2`) and the absent ID `forgejo-push-delivery-absent-004` from the retained, verified local inbox. It ran the CLI under Bubblewrap `0.9.0` with `--unshare-net`. A local TCP listener on host `127.0.0.1:3130` served as a positive control: the host process connected successfully. Inside the sandbox, `/proc/self/ns/net` had inode `4026535871` versus host inode `4026531833`; the sandbox loopback probe could not connect, and the host listener received zero sandbox connections.

The CLI exited 1 with `inconclusive/delivery_not_found`, `retryable=false`, the absent UUID in the action details, and CLI-reported `upstream_read_attempts=0`. A synthetic credential canary passed through an internal pipe was still fully present after process exit; only `credential_preserved=true` was saved. SHA-256 before/after of the webhook inbox matched. This independently corroborates network denial at the OS namespace boundary alongside the credential and journal checks.

## Reproduction and verification

The evidence is reproducible with `run_reconcile_batch_network_isolated` and `validate_partial_push_batch_refusal` in `scripts/run_forgejo_native_push_webhook_reconcile.py`, using the retained `/tmp/forgejo-native-push-20261025-004` state and the delivery IDs in `result-with-journal-check.json`. Bubblewrap mounts the binary read-only and only the local `data` directory writable so the journal lock can be acquired; it clears the environment, hides `/home`, `/root`, `/run` and `/tmp`, and creates a new network namespace. No host descriptor is passed to the sandbox. The synthetic canary is supplied to the supervisor over stdin, then the supervisor creates the private credential pipe for the CLI.

The test-first run initially failed because the observation validator was absent; a later test-first refinement failed because the journal-integrity argument was not implemented. The final Python suite passes 43/43, Python byte-compilation passes, Cargo license guard tests pass 7/7 with the license still unselected, and `git diff --check` passes. The preceding lot’s full offline Rust suite remains 510 passed, 0 failed, 61 ignored; this follow-up changed no Rust source.

## Limits

This is a local network-namespace proof, not a complete filesystem sandbox or a production security certification. The harness gives the trusted reconciler read-only visibility of the host root outside the hidden home/runtime/temp paths and writable access to the private test data directory for its lock file. It does not test arbitrary hostile executable code, multi-tenant isolation, PR/merge authority, AgentOS/LLM execution, or comparative productivity. No external network connection, model call, credential service, publication, or production change occurred.
