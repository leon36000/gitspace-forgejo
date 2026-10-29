# Append-only manifest reconciliation — CONTINUATION-437

The first local receipt append passed the worktree index, but the parent project index had 52 current workspace paths whose latest indexed hashes were stale. The paths were all under `.runtime/webhook-005-workspace/`; the pre-repair verifier reported no missing, malformed, unsafe, or unhashed entries. The names and old/current SHA-256 values are recorded in `parent-manifest-reconciliation.json`.

`repair_parent_manifest.py` appended the current hash for those 52 paths. It changed no source or evidence payloads. The previous project-index prefix of 12,788 lines, SHA-256 `e008badaddc581f4a47d88f66dcf0efd3de447673b477c2136e0a3cca6e93134`, is preserved. The project index then verified 8,854/8,854 unique paths with zero mismatches.

Next, `finalize_manifests.py` appends this reconciliation receipt to the local, worktree and project indexes, then the three verifiers are rerun. These indexes attest the latest recorded bytes; they do not prove content correctness, provenance or absence of unrecognized secrets.
