# GitLab PC4 transfer progress — 28 octobre 2026

At branch commit `36c2c9fc3db051cd76d251a44b44d4ef37c9a88b`, commits `64615249daf75a6533ece4a7c341a3f0b4971495` through `36c2c9fc3db051cd76d251a44b44d4ef37c9a88b` import the first 7,000 of 14,715 candidate-manifest rows, in 70 create-only batches of 100 files. Before each upload, every local file in the batch was re-read and checked against its Git blob SHA-1, expected size, UTF-8 round-trip, and file mode.

Remote tree readback remains 400/400 exact for rows 1–400; rows 401–7,000 have not yet been individually read back. Latest observed pipeline 231 on the prior checkpoint was `skipped`; the current batches also use `[skip ci]` and need an unsuppressed full pipeline after import.

The eight TRANSFER-006 control files, the current checkpoint and global SHA256SUMS, and this progress receipt are outside the candidate list and remain to be synchronized. No MR/default-branch change; no email or PAT was read or used. Code execution and tests remain on PC6; GitLab writes use the authenticated connector.

Next: continue at candidate row 7,000, then reconcile every candidate blob and mode against the final recursive GitLab tree and run CI on the exact final commit.
