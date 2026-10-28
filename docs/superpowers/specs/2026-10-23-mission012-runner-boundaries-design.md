# MISSION-012 Runner Boundary Hardening — Design

## Intent

Harden the local composed admission-to-PR harness after a read-only review found that build inputs, process lifetimes, and Claude output capture were not consistently bounded. The goal is to make the existing local demonstration safer and more reproducible without changing Forgejo or claiming production isolation.

## Approaches

- Keep the runners as-is and downgrade the E2E security claims. This would leave known credential exposure and orphan-process paths in the local harness.
- Patch the existing runners and tests at the concrete boundaries. This is the selected approach because each review finding has a small observable fix and the current architecture already has a stage process group, owner-only state, and focused Python tests.
- Replace the harness with cgroups/containers and a new supervisor. That would broaden the change substantially and is not required to close the demonstrated process-group escapes in the current local runner.

## Scope and constraints

- Build from an explicit manifest/lock/source snapshot that excludes `.runtime`, credentials, logs, and evidence.
- Require `enable_push=false` in every readback of the protected branch rule.
- Keep the receiver, Forgejo fixture, MCP host, and model child under a cleanup boundary; handle timeout after stdout EOF.
- Stream Claude stdout/stderr under a strict combined byte cap and terminate the child process group on timeout, overflow, or parent cancellation.
- Add focused regression tests and run them in a staging copy that contains code/tests only. Do not run another live model E2E in this slice.
- Preserve all existing dirty files; no commit, network publication, production access, credential use, or cleanup of prior state.

## Limits

This improves the disposable local harness; it does not isolate hostile code sharing the same UID, make Forgejo's merge API base precondition atomic, or demonstrate a human review or general project benefit. The configured Claude CLI may receive only the selected source and test files, never runtime state or credentials.
