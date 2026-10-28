# Forgejo v17 LFS ref-fence regression — POC-072

## Scope

Scratch-only regression check against Forgejo source based at `0d791421f85ca13a63b665053d81ea765fbbc571`. The test is an existing integration test from the experimental LFS staging work. It exercises `services/repository/files.ChangeRepoFiles` through a real temporary-repository Git push and the local post-receive hook. The helper calls the service directly; this is not an HTTP API route test. No Forgejo source was copied into the product worktree.

## Run and result

Command: see `commands.txt`. Tool versions and source SHA-256 values are recorded beside this report. The command exited 1 (recorded in `exit-code.txt`). This is a regression witness: the test expected LFS metadata to remain pending after a seam moved the target branch ref back to its base commit during post-receive, but observed that the LFS association was already active and the pending set was empty.

The post-receive hook checks that the target branch still equals the candidate before promotion (`routers/private/hook_post_receive.go`, around the generic LFS promotion block). The experimental `ChangeRepoFiles` service then promotes the operation unconditionally after `t.Push` (`services/repository/files/update.go`, immediately after the push). Thus the hook's ref check is bypassed by the service-level promotion. The service returned a non-nil `FilesResponse` for the candidate while the target branch was back at the base commit. The two failed assertions are the active-association lookup unexpectedly succeeding and pending-row count being 0 instead of 1.

This result establishes the specific regression in this dirty experimental scratch tree. It does not establish behavior of unmodified upstream Forgejo, a live HTTP API, other writers, other databases or a complete shared fence. It also does not establish atomicity for a future read-ref-then-promote check; that would remain vulnerable to a concurrent ref change unless synchronized with every writer.

## Evidence files

- `test.log`: full output from this one targeted run.
- `baseline.txt`: audited source commit and dirty scratch state.
- `source-hashes.txt`: exact source snapshots used for the interpretation.
- `toolchain.txt`: Go and Git versions.

## Decision and next action

Do not adopt the unconditional service promotion. Prototype acknowledgement gating that preserves staged state when the target ref has moved, then identify and test one shared synchronization/fencing mechanism across every ref writer before describing promotion as safe. Keep all Forgejo experiments in scratch pending a separate architecture and maintenance decision.
