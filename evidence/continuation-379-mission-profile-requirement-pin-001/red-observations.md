# Test-first observations

These are concise observations transcribed from the process-runner output of this session; the complete successful final output is in `mission-profile-process-e2e-final.log`.

1. Before adding the profile digest check, the wrong but well-formed caller digest returned `webhook_inbox_missing`; the test expected `mission_profile_requirement_mismatch`. This proves parsing had not established the profile's digest authority before inbox access.
2. The changed-requirements scenario initially stopped at `mission_source_invalid_response` because its synthetic tree omitted the `sha` required by the already-implemented CONTINUATION-377 contract. After adding the served commit SHA to the fixture, the unchanged runtime admitted the modified same-revision requirements, returned `verdict=claimed` and `execution_generation=1`, and wrote the admission handoff. This is the intended red result.
3. After the code change, both refusals pass before journal allocation/handoff, and the unchanged profile's positive admission still passes.
