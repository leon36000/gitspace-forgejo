# FORGEJO-NATIVE-PUSH-PARTIAL-SELECTION-005

Date: 2026-10-25. Local-only execution in the disposable Forgejo instance rooted at `/tmp/forgejo-native-push-20261025-004`.

## Question and observed behavior

Can the reconciler reject a batch containing one actual Forgejo push delivery and one absent delivery ID without consuming the host credential or reading upstream state?

The runner pushed twice by Git HTTP to the private local Forgejo 17.0.0 repository `forge-admin/agent-forge-core`, branch `agent/native-push-proof`. Forgejo emitted two distinct native `push` deliveries and recorded successful webhook tasks (HTTP 202). The first push created the branch at `152232adde1440dab03a85f79cc53ab0a2148934`; the second advanced it to `e2acaf2297fbb4adfa315692ca3c9a7cd82287fe`.

The normal batch reconciled both real delivery UUIDs to the current head with `upstream_read_attempts=1`; the host credential was consumed from the private pipe and the inbox journal remained byte-for-byte unchanged. The older notification still names the first commit, so this also checks stale and current push notifications in the same batch.

The refusal call requested the first real UUID and `forgejo-push-delivery-absent-004`. It exited 1 with schema v5 `inconclusive/delivery_not_found`, `retryable=false`, and an actionable instruction to confirm inbox admission. The response identified the absent ID, reported `upstream_read_attempts=0`, and the credential pipe still contained the exact credential bytes after process exit. Only the boolean `credential_preserved=true` was recorded; credential contents were neither logged nor serialized. The journal remained byte-for-byte unchanged after refusal.

## Commands and evidence

Successful end-to-end command:

```text
python3 scripts/run_forgejo_native_push_webhook_reconcile.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-native-push-20261025-004 --evidence-dir evidence/forgejo-native-push-webhook-reconcile-005
```

Forgejo binary SHA-256: `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; version `17.0.0+gitea-1.22.0`, built with Go `1.26.9`. The repository source HEAD was `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. The evidence JSON records source, binary, receiver and CLI hashes, webhook task bindings, API branch readbacks and cleanup. Build used Bubblewrap `0.9.0`, `cargo build --offline --locked`, and a cleared environment with `--unshare-net` for the build.

Tool versions: Rust/Cargo `1.95.0`, Python `3.12.3`, Git `2.43.0`, Bubblewrap `0.9.0`. Verification outputs are included here: Python suite 38/38; `py_compile`; Cargo suite 510 passed, 0 failed, 61 ignored across 35 test groups; Cargo license-state guard 7/7. The guard reports `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, and redistribution rights `not_assessed`. `git diff --check` passed.

The first runner invocation was refused before starting Forgejo because the chosen evidence directory already contained test logs. That refusal is retained in `first-attempt-refused.txt`; the successful run used a new evidence directory. No service was left listening: Forgejo and receiver stopped with return code 0, the webhook was disabled, and post-stop listener checks were empty. The isolated `/tmp` state is retained; nothing was deleted.

## Interpretation and limits

This demonstrates the refusal against a real Forgejo-emitted push delivery and the host credential pipe at the local CLI process boundary. The zero-upstream count is reported by the reconciler and is consistent with the source path that selects every requested delivery before reading the credential or constructing upstream calls. This run did not capture packets independently, stress concurrency, exercise a PR or merge, invoke AgentOS or an LLM, or measure a productivity gain. It is one repository and one two-push sequence, not evidence of production or multi-tenant security.

No Forgejo or GitSpace source was copied into the product. GitSpace redistribution rights remain undetermined. No product license was selected. No external model call, publication, production change, or external credential use occurred.
