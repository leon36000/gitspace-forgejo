# GitLab PC4 transfer progress — 28 octobre 2026

At branch commit `8497dae116d25c1a98315e1b9342733b64e9c61d`, commits `64615249daf75a6533ece4a7c341a3f0b4971495` (batch 001) through `8497dae116d25c1a98315e1b9342733b64e9c61d` (batch 054) contain the first 5,400 of 14,715 candidate-manifest rows. Each commit adds 100 new paths. Every local source file was re-read and checked against its Git blob SHA-1, expected byte length, UTF-8 round-trip, and mode before submission.

Remote recursive-tree readback matched paths 1–400 by blob ID and mode (400/400). Rows 401–5,400 still require remote readback. Intermediate commits use `[skip ci]`; latest observed pipeline 231 for the current head is `skipped`. A full unsuppressed pipeline is still required on the final import SHA.

Still outside the candidate manifest: the eight TRANSFER-006 control files, current `docs/checkpoint.md`, `evidence/SHA256SUMS`, and this progress receipt. These must be added to the private work branch after the import state is finalized. No merge request/default-branch change; no email/PAT use. PC6 remains the execution environment; GitLab mutations use the authenticated API connector.

Next: continue from row 5,400 using create-only batches, then perform the full tree/blob/mode reconciliation and run CI on the final SHA.
