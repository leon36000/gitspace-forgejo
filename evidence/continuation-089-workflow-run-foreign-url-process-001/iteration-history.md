1. Added the ignored receiver+CLI process scenario and allowed it in the fixed-scenario Bubblewrap driver. The focused process run passed 1/1 against a loopback HTTP fixture.
2. The first `cargo fmt --all -- --check` found one rustfmt layout difference in the new assertion. `cargo fmt --all` corrected formatting; no behavioral test failed.
3. Added a combined stdout/stderr secrecy assertion, reran the focused process scenario, and reran all-targets after formatting. The final focused process scenario passed 1/1; the final offline suite passed 597/0/94 across 34 test executables.
4. Final rustfmt check, `git diff --check`, runner py_compile, and local/global append-only manifest checks passed.
