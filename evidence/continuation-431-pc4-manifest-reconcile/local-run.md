# CONTINUATION-431 — readback et réconciliation du manifeste PC4

Date: 2026-10-28. Scope: read-only readback plus one append-only update to the private GitLab work branch.

## Remote identity

- Project: `root/gitspace-forgejo` (GitLab PC4 project ID 4, private).
- Work branch: `codex/pc6-transfer-work-2026-10-28`; default/protected branch was not modified.
- Previous commit: `d72f6077369853bb84256c45b455a856fe7a89a8` (CONTINUATION-430).
- Manifest-reconciliation commit: `c4a3a353c4e41ce756459045aa6fff0e711a8657`.

## Evidence and action

The GitLab tree returned the same Git blob IDs as local `git hash-object` for all 16 selected CONTINUATION-430 files (two source/test files, two docs, and twelve evidence files). A paginated scan of the remote `evidence/SHA256SUMS` found that the latest remote digests agreed for the 14 source/evidence files, but not for the two documents. The local current SHA-256 values were `246ab0e180334a457011f02ce77bd70d116cc416c3856ff515e0cad72d57452a` for `docs/architecture.md` and `8a921717ab3e8f041bc42c8801386cf0ad5166c11c7f6b651c1ab0b79def716d` for `docs/checkpoint.md`.

Commit `c4a3a353…` appended those two records to the remote global manifest. Readback at that commit showed both records as the final two lines. No source file, binary, credential, or default-branch ref was changed by that commit.

Local verification command `python3 -B scripts/verify_latest_sha256_manifest.py evidence/SHA256SUMS --root .` exited 0: 13,500 unique paths verified from 18,511 valid records; zero malformed, missing, mismatched, unhashed, or unsafe paths. This is a local manifest result; the remote repository does not contain the full local worktree.

The latest pipeline observed before this evidence follow-up was pipeline 162 on `c4a3a353…`, status `pending`; it is not a passing result for the subsequent commit. Pipeline results must be read against the exact commit SHA.

## Limits

This reconciles hashes for selected transferred files only. It does not prove full worktree/history transfer, an LLM run, a benchmark, or a product gain. The local worktree remains heavily modified. GitSpace redistribution rights remain undetermined; no GitSpace or Forgejo source was copied into the product.
