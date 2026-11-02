# Gitleaks triage

Source reports: job 627 scanned commit `9ac877767e9b458a9cd911e67a10f87aeeaefbdb` and produced 988 redacted detections; job 5316 scanned commit `787c6e04e7ff8c1f26d47644a8fae4b4d6f0452a` and produced 3 additional redacted detections. Total classified: 991.

No credential values are included. Every detection is mapped to its exact fingerprint, file, commit, line, classification, and rationale in `GITLEAKS_TRIAGE.csv`. `.gitleaksignore` contains only the exact finding fingerprints listed there; no rule-wide, file-wide, or path-wide suppression is used.

| Classification | Findings | Rationale |
|---|---:|---|
| content-checksum | 5 | Source/content SHA-256 or object identifier used as integrity metadata |
| git-commit-id | 6 | Commit/API SHA reference, not a credential |
| idempotency-key | 969 | Opaque request-deduplication identifier; no authentication role in source context |
| public-signing-fingerprint | 1 | Public key fingerprint metadata |
| token-file-checksum | 10 | SHA-256 digest of a local Forgejo test-token file path; only checksum metadata is committed |

The source review found no raw authentication credential. Forgejo test-token file names appear only as SHA-256 digests in project-state evidence; the other detections are idempotency identifiers, commit references, public key fingerprint metadata, or content integrity hashes.
