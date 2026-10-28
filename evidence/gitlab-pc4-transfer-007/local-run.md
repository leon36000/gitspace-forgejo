# GitLab PC4 transfer progress — 28 octobre 2026

## Scope and source

Private project `root/gitspace-forgejo` (project ID 4), work branch `codex/pc6-transfer-work-2026-10-28`. The local PC6 audit in [TRANSFER-006](../gitlab-pc4-transfer-006/local-run.md) compared the worktree with branch baseline `796a6085cb71cd7d2ae4c3be7c53c3d401691448` and selected 14,715 UTF-8 text evidence paths (58,717,295 bytes) absent from that baseline. The audit excluded runtime state, binaries/archives, invalid UTF-8, the TRANSFER-006 control directory, and 108 source snapshots; its six known secret-pattern families had no matches, which is not an exhaustive privacy/license review.

## Progress recorded

Commits `64615249daf75a6533ece4a7c341a3f0b4971495` through `b180d1b1c354ead33fbd0b8af64a697357aef8c4` import candidate-manifest rows 1–1,400 in 14 create-only commits of 100 paths each. The local file contents were checked against Git blob SHA-1 and mode metadata before each batch. A GitLab recursive-tree readback at `953dda3e1b43d5acb9bd2c840dc0eab2593fd0e6` matched rows 1–400 by blob ID and mode (400/400); rows 401–1,400 still need remote readback. Bytes/paths are not yet fully reconciled against the final branch.

GitLab reports the pipelines for these intermediate pushes as `skipped` (latest observed pipeline 191, SHA `b180d1b1c354ead33fbd0b8af64a697357aef8c4`), consistent with the `[skip ci]` commit messages. A complete, unsuppressed pipeline remains required on the final import commit.

The eight TRANSFER-006 control files, current `docs/checkpoint.md`, and `evidence/SHA256SUMS` are outside the 14,715-row candidate manifest and remain to be synchronized after progress is finalized. The source code/tests/docs in the baseline audit were already present on PC4 except these tracking updates. No merge request or default-branch update was made; no email or PAT was read or used. Local verification and execution remain on PC6, and GitLab writes use the authenticated connector.

## Next

Continue from candidate-manifest row 1,400 using create-only batches, guarding the observed branch head and checking each local blob/mode before upload. At the end, compare all 14,715 paths against a recursive remote tree readback, synchronize the control documents, and run/read back a full pipeline for the exact final SHA.
