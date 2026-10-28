# CONTINUATION-289 — locked Cargo dependency license metadata inventory

Date: 2026-10-27. Product worktree base HEAD: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; the worktree is heavily modified. This lot only adds a sourced metadata inventory and updates the existing source inventory and checkpoint; it does not modify runtime code, Cargo metadata, Forgejo, or any service.

## Result

`cargo metadata --locked --offline --format-version 1` exposes 40 package records: one local workspace package, `agent-forge-core 0.1.0`, and 39 registry packages. The exact package/version/source and raw Cargo `license` strings are in `packages.csv`; counts and lockfile identity are in `summary.json`.

The local `Cargo.lock` hash is `459fef3749a42c989521901dcde61cb26e12ed4206c5d722e149426c2d3a5439`; `Cargo.toml` hash is `a2c1182d6e5cc272c50fb0ad5f16d210befcd452527d29e4dde6cd6b05206d09`. For every one of the 39 registry packages, the SHA-256 of the cached `.crate` archive matches the checksum recorded in `Cargo.lock`. Every corresponding extracted source tree has at least one recognized top-level license file; `registry-source-provenance.json` records file names, hashes, raw metadata, and per-package checksum comparisons.

The raw metadata contains 26 `MIT OR Apache-2.0`, 3 `Apache-2.0 OR MIT`, 3 `BSD-3-Clause`, 2 `MIT`, 2 `MIT/Apache-2.0`, and one each of `(MIT OR Apache-2.0) AND Unicode-3.0`, `MIT OR Apache-2.0 OR BSD-1-Clause`, `Unlicense OR MIT`, and `UNDECLARED`. The undeclared package is the local `agent-forge-core`; the crate license remains unselected. The slash form is preserved as supplied and has not been normalized or parsed as SPDX.

## Reproduction

Run from the product worktree, offline:

```text
python3 evidence/continuation-289-cargo-dependency-license-inventory-001/generate_inventory.py
python3 evidence/continuation-289-cargo-dependency-license-inventory-001/verify_registry_sources.py
```

The first helper runs Cargo metadata and writes the CSV/summary. The second checks each cached archive against the lockfile checksum and hashes recognized top-level license documents from the extracted crate source. Both ran successfully: 40 metadata packages, then 39/39 archive checksums and 39/39 package roots with recognized license documents. Toolchain and exact commands are in `tool-versions.txt` and `commands.txt`.

## Scope and limits

This is an inventory of the current product Cargo resolution only, using the local offline registry cache. It preserves upstream-declared license metadata and verifies the integrity of the cached source archive against the lockfile. It does not parse or review the contents of all license texts, validate every raw string against an SPDX version, assess compatibility or notice obligations, determine ownership, or authorize distribution. The root package has no declared `license`/`license_file`; the existing project policy keeps publication disabled and distribution rights unassessed. Forgejo is consumed as a separate binary, so its Go dependency tree is not in this lockfile. No GitSpace code is reused, and this report does not audit GitSpace's hypothetical dependency closure.

No software tests were run for this documentation/inventory lot. The previous CONTINUATION-288 runtime suite evidence is separate; no new runtime behavior or productivity gain is claimed. No model, external service, secret, deployment, publication, or production environment was used.
