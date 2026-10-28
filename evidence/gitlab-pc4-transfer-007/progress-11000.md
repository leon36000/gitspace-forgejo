# GitLab PC4 transfer progress — 28 octobre 2026

At branch commit `27756bc5cca9d9498b19bf735bc104167cd8c013`, commits 001–110 import the first 11,000 of 14,715 candidate-manifest rows in 110 create-only commits of 100 files. Each upload re-reads local files and validates Git blob SHA-1, byte count, UTF-8 round-trip, and mode.

Remote recursive-tree readback is still 400/400 exact for rows 1–400; rows 401–11,000 need final remote reconciliation. Latest observed pipeline 287 for the current SHA is `skipped`. Run a full pipeline without `[skip ci]` on the final import SHA.

The TRANSFER-006 audit control files, current checkpoint and global manifest, and progress receipts are outside the candidate manifest and still need import. No MR/default-branch change; no email/PAT use. Tests remain on PC6 and GitLab writes use the authenticated connector.

Next: continue with candidate row 11,000, then perform complete remote blob/mode reconciliation, sync control records, and verify the final unsuppressed pipeline.
