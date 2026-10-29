# GitLab PC4 final transfer readback and CI — 28 octobre 2026

## Repository transfer

Private project `root/gitspace-forgejo` (ID 4), work branch `codex/pc6-transfer-work-2026-10-28`. At commit `9ac877767e9b458a9cd911e67a10f87aeeaefbdb`, a recursive tree readback returned 163 pages / 16,202 entries / 15,174 blobs. All 14,715 paths in the TRANSFER-006 candidate manifest matched their expected Git blob IDs and modes; all 15 audit/control files also matched. No missing or divergent paths. The branch default remained `6c6ffa3d7d37d48b9d7877acb982d73a2c49a14f`; no MR was created. Tests remained on PC6; GitLab was accessed through its authenticated connector, with no email or PAT read.

The initial export scan selected only UTF-8 text evidence absent from the baseline. It excluded `.runtime`, `.claude`, archives/binaries, invalid UTF-8, and 108 source snapshots. Its six known-secret-pattern checks found no matches, but that heuristic did not establish absence of all credentials.

## CI result and secret-scanner review

Pipeline 328 ran on the exact transfer commit above and ended `failed`. `pc4_semgrep` (job 628) and `pc4_trivy` (job 629) succeeded; `pc4_gitleaks` (job 627) failed; `pc4_ci_fallback` (job 630) was skipped. Gitleaks v8.30.1 reported 988 `generic-api-key` findings in 336 evidence outputs. Its JSON artifact had both `Secret` and `Match` redacted for all 988 records; no values are reproduced here.

A local, value-redacted classification found 978 findings on `idempotency_key` fields, consistent with high-entropy identifiers; it found 10 findings on fields named `forgejo-read.token` and `invalid-forgejo-read.token`, each 64 characters, across five output files. No known GitHub/GitLab/OpenAI/AWS credential prefix appeared. Source `scripts/run_mission_012_live_admission_e2e.py` generates a bounded read-only token through the local Forgejo admin command and stores it under the test project `.runtime`; this does not prove the token was revoked after the test, nor prove that the copied evidence values are harmless. Treat the ten token-shaped values as unresolved until the user confirms their test-only scope or requests revocation/redaction.

No retry, history rewrite, deletion, or merge was performed. Because the full-tree scan has already completed and the credential classification needs a human decision, further GitLab writes and cleanup of these values are on hold. Continue independent local work. If history removal is requested, obtain explicit confirmation because rewriting the branch removes/replaces existing Git history.
