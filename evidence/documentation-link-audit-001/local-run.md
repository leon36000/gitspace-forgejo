# DOCUMENTATION-LINK-AUDIT-001 — références locales des documents canoniques

Date: 2026-10-28. Product worktree base HEAD: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; the worktree is heavily modified.

The scan covers simple Markdown links in `docs/checkpoint.md`, `docs/architecture.md`, `docs/implementation-plan.md`, `docs/refusal-evidence-matrix.md`, and `docs/source-inventory.md`. The pre-edit result found 1,602 links and three missing targets: the CONTINUATION-289 and CONTINUATION-297 receipts existed but their relative links omitted `../evidence/`; the third target was an ephemeral `/tmp/forgejo-merge011-fix-test/CHECKPOINT.md` absent from this host. The two evidence links now resolve. The stale temporary-file link was replaced with a note that the checkpoint is unavailable and a link to the retained MERGE-011 report.

The post-edit script run is recorded in `after.json`; it checks 1,602 local links and reports zero missing targets. `git diff --check` and the latest-record global SHA-256 manifest verifier pass. Exact document scope and commands are in `verify_doc_links.py` and `commands.txt`; file hashes are in `SHA256SUMS`.

Limits: the small Markdown-link parser handles ordinary inline links only; it does not validate anchors, reference-style Markdown links, external URLs, or the truth of evidence contents. This lot changes documentation and adds a local audit script/receipt only. No runtime code, Forgejo/GitLab service, remote branch, credential, deployment, or production system was modified or contacted.
