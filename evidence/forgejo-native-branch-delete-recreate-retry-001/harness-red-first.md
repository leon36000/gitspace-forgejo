# Harness red-first note

The first live invocation completed the initial inconclusive batch, disabled the hook, recreated the branch, and reached the retry path. The run then stopped in evidence serialization with `NameError` because the JSON writer referred to the old local name `retry_status` after the result had been stored as `retry_exit_status`. The runner's `finally` path confirmed the hook disabled and both Forgejo and receiver listeners stopped.

The variable reference was corrected. The failed receipt remains unchanged under `live-run/`; the successful rerun used a fresh private state directory and is recorded under `live-run-002/`. The failure was in the test harness and does not count as a Forgejo or adapter refusal result.
