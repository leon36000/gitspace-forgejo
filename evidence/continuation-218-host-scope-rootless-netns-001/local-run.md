# CONTINUATION-218 — host-created scope with rootless network namespace

## Finding

The previous diagnostic used `unshare --net` without a user namespace and correctly observed `EPERM`. This probe tested the distinct operation `unshare --user --map-root-user --net` from a process whose transient systemd scope had first been prepared and attested under the host UID.

The allowed child command completed successfully. The host supervisor attested the waiting scope before sending the command envelope; the child entered a distinct network namespace, mapped its own UID/GID 0 to host UID/GID 1000, and remained in the same attested cgroup. The outer scope cleanup returned `clean`.

Inside the rootless user namespace, `/usr/bin/systemd-run` and `/usr/bin/systemctl` appeared with UID 65534. The existing `_trusted_systemd_binary` check rejected both, and `prepare_process_scope` refused before launching its requested `/usr/bin/true` command. Thus a host-created outer scope can contain a rootless network-namespace child, but the current in-namespace runner still cannot create its normal nested stage scopes.

The complete observation is [scope-probe.json](scope-probe.json). The exact probe source, commands, tool versions, and source hashes are in this directory.

An initial harness invocation failed after scope startup because `Popen.communicate()` tried to flush the stdin channel that `launch_scoped_process` had already closed after attestation. Its exception path invoked `_abort_scope_process`; that attempt is not counted as a successful run, and its cleanup was not separately recorded. The corrected probe sets the already-closed `process.stdin` field to `None` before `communicate()`; the reported run is the corrected execution with explicit `cleanup: clean`. The initial observation is retained in [failed-initial-harness-attempt.txt](failed-initial-harness-attempt.txt).

## Scope and limits

This verifies one local process/cgroup/namespace lifecycle primitive on Linux aarch64, util-linux 2.39.3 and systemd 255. It did not run Forgejo, an agent, a gate, an API request, a candidate, a credential, or a verifier. It does not demonstrate that the current composition works in this launch mode, that an agent cannot access Forgejo, that only gate traffic is allowed, or that a host broker would expose a safe narrow interface. It is not a production security claim.

The observation narrows the launcher design: create and attest the outer scope before entering the rootless user/network namespace; keep the current root-owned-binary check; do not give the agent the systemd user bus or merge credential. The next proof must adapt the current supervisor to either a verified outer-scope lifecycle or a narrowly allowlisted host-side stage broker, then test API/Web/receive-pack and credential refusal while gate read/merge requests still work. Cancellation, time/resource ceilings, and cleanup must be tested on that composed boundary before adopting it.

No model was called and no cost incurred. The project remains incomplete; no product code or upstream Forgejo code was changed by this diagnostic.
