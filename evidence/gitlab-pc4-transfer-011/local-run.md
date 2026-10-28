# GitLab PC4 transfer readback — 28 octobre 2026

## Scope

Private project `root/gitspace-forgejo`, work branch `codex/pc6-transfer-work-2026-10-28`. The baseline snapshot was `796a6085cb71cd7d2ae4c3be7c53c3d401691448`; code/tests/docs were already present there except the previously recorded tracking changes. TRANSFER-006 selected 14,715 UTF-8 evidence files absent from that baseline, totaling 58,717,295 bytes. Its known-pattern scan found no matches for six secret-pattern families; it is heuristic, not an exhaustive privacy or license review. Runtime and Claude state, binaries/archives, invalid UTF-8, and 108 source snapshots were excluded.

## Candidate file readback

All 14,715 candidate paths were imported by create-only commit batches 001–148. Each local source file was re-read and checked against the candidate Git blob SHA-1, size, UTF-8 round-trip, and mode before upload. On candidate-import commit `20af706fc820814aacd0a7c303c75bb3b66f2afb`, GitLab returned 161 pages / 16,032 entries under `evidence/` (15,023 blobs). Comparison against all 14,715 manifest rows found 14,715 exact blob-ID and mode matches, zero missing files, and zero mismatches. The branch head remained that SHA for the duration of the readback. This proves the selected evidence paths, not that excluded local state or every private/third-party detail is safe to redistribute.

Intermediate push pipelines were marked `skipped` (latest observed before this readback: pipeline 287 on SHA `27756bc5cca9d9498b19bf735bc104167cd8c013`). The final checkpoint, global manifest, and transfer audit controls are being synchronized separately; an unsuppressed pipeline and final branch-tree check remain required after that control commit. No merge request or default-branch update was made. No email/PAT was read or used. Tests remain on PC6; GitLab changes use the authenticated connector.
