# Append-only manifest correction

The initial `source-sha256.txt` and its local index entry are retained exactly as recorded. Before that first append, the refusal-matrix row was accidentally concatenated to the previous row; the local markdown row break was then corrected. The old matrix SHA-256 in the historical file is `a1a8ada9721dd7aae7c3f01043cc3b087975488203151b552d34204bb89e182c`. The corrected current matrix SHA-256 is `cfeb5eeb4490fc84ce243a0b81b9dd8e175b0fbc62a63409b7dd04a686cd1827`.

`source-sha256-latest.txt` lists the current hashes for all 21 source/evidence paths. The append-only worktree and project manifests add the corrected matrix hash and refreshed source list; earlier entries remain intact. No run receipt or source code changed.
