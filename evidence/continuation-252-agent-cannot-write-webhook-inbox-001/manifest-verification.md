# CONTINUATION-252 — append-only manifest verification

The first append helper wrote the local evidence hashes and appended the corresponding worktree/project rows. Its final verifier then failed while parsing historical blank lines in `evidence/SHA256SUMS`; the worktree index also contains one older `None` digest row. No index rows were removed or rewritten.

This independent verifier ignored blank and malformed historical rows only for lookup; it preserved them byte-for-byte. Before and after the final append, the original worktree and project manifest prefixes match the saved count/SHA-256 receipt. Latest entries for the local evidence, source files, local index, and referenced worktree index were re-read and matched.

Historical index anomalies observed at verification time: worktree blanks=6, malformed rows=1; project blanks=2, malformed rows=0. These are retained as historical data and are not treated as valid hashes.
