# TDD red observations

1. The new held-gate process test expected a host-generated `host_observed_agent_process_tree`. Before implementation the process completed, but the wrapper result omitted the field; assertion failed as intended.
2. The result-validator test failed before implementation because `validate_host_observed_agent_process_tree` did not exist.
3. After adding PID/network checks, an expanded test failed because mount namespace, user namespace and cgroup membership were not yet emitted. A first implementation also assumed Bubblewrap shared the user namespace; a direct `/proc` read showed a distinct user namespace with the same mapped effective UID. The final contract now requires a distinct user namespace and the observed same UID.

An exploratory anonymous-pipe `/proc/<pid>/fd/<fd>` probe had a positive host-side control, but the Bubblewrap child could not read the parent FD both with and without `--unshare-pid`. The negative control therefore did not discriminate PID namespace isolation, likely because of the host ptrace policy; it is explicitly excluded as proof. No real credential or secret was used.

One instrumentation attempt initially failed before starting Bubblewrap because the evidence runner resolved the workspace path one parent too high; its import path was corrected and the reproducible runner then passed. This was a harness-path failure, not sandbox evidence.
