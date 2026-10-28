# LFS acknowledgment waits for post-receive publication — POC-073

## Scope and change

Scratch-only experiment on Forgejo v17.0.0 source based at `0d791421f85ca13a63b665053d81ea765fbbc571`. The previous POC-071 service-level unconditional promotion bypassed the post-receive branch-ref check and failed the POC-072 ref-move regression. In this experiment `services/repository/files.ChangeRepoFiles` no longer publishes staged metadata itself. After `t.Push` returns, it checks the operation-scoped pending-row count and confirms each expected OID has an active repository association. Pending rows or missing active metadata return an actionable recoverable error containing the candidate commit and operation key. Normal publication stays in the post-receive hook, which checks that the target ref points to the candidate before promoting.

The source and test snapshots are identified by SHA-256 in `source-hashes.txt`; the scratch HEAD is the upstream v17.0.0 base above, with the prior experimental modifications preserved. No Forgejo source was copied into the product or deployed.

## Verification

The final targeted run in `final-run.log` passes 4/4 integration tests with SQLite and the explicit POC build tag:

- The ref-move seam makes the hook leave the LFS operation pending; `ChangeRepoFiles` returns an error and no `FilesResponse`; the association remains hidden and GET returns 404.
- The separate reflog recovery case confirms the existing recovery policy can later promote an accepted candidate recorded in the target ref reflog, even after the ref moved back.
- The real HTTP contents route returns 201 only after the normal post-receive hook has activated the LFS association; immediate LFS GET returns the original bytes.
- An injected SQLite insertion failure makes post-receive leave a durable pending operation; the route returns 500, GET returns 404, and recovery succeeds after the trigger is removed, with GET returning the original bytes.

`gofmt -d` produced no output and `git diff --check` passed; captured outputs are empty in their files. Full test output includes expected forced SQLite/log errors from the injected failure scenario.

An earlier experimental invocation is retained as `intermediate-run.log`: it exited 1 because it omitted the POC build tag and retained assertions expecting recovery to defer despite the candidate being recorded in the target ref reflog. The test was narrowed to assert the actual acknowledgment boundary; the separate reflog test remains and passes. Only the final four-test run is the passing verification claim.

## Limits and next action

This closes the specific POC-071 regression and demonstrates service/HTTP acknowledgment gating in the SQLite integration harness. It does not prove a shared fence across every Git ref writer, atomicity between the post-receive ref read and SQL publication, behavior on PostgreSQL or distributed storage, or compatibility with all API/SSH/web writers. A future writer could still move a ref in the interval after the hook reads the ref and before the DB transaction commits. Continue by inventorying writer paths and testing concurrent ref updates at that boundary. Do not adopt this patch without a Forgejo maintenance and architecture decision.
