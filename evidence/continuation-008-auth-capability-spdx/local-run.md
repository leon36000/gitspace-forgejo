# CONTINUATION-008 — contrat d’accès privé découvrable et statut SPDX non choisi

Date : 25 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD observé au début et après le lot : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le checkout était déjà fortement modifié et aucune modification existante n’a été réinitialisée, commitée ou publiée.

## Changement

- `forgejo-reconcile-local` rend l’action `resource_not_found` explicite sur l’ambiguïté observée : un 404 peut masquer une ressource privée; il faut confirmer l’existence et la visibilité du credential hôte, dont la valeur reste hors de l’entrée agent.
- `--capabilities` publie un `credential_contract` additif sur le drapeau `--host-credential-fd`, la liaison à l’origine, l’absence de credential dans argv/environnement/journal/entrée agent, et la conséquence du 404 privé observée dans le dossier `FORGEJO-V17-LIVE-PRIVATE-PR-READBACK-001`.
- `Cargo.toml` garde `license` et `license-file` absents et `publish = false`. La métadonnée ignorée par Cargo `spdx_package_license_declared = "NOASSERTION"` sert seulement de marqueur pour un futur export SPDX. Ce n’est ni un choix de licence, ni une déclaration que le code n’a aucune information de licence, ni une autorisation de redistribution.

## TDD et vérifications

Avant l’implémentation, le nouveau test Python a échoué (métadonnée export absente; valeur incorrecte non rejetée) et le test Rust a échoué parce que l’action 404 ne mentionnait pas la visibilité privée. Après l’implémentation :

- `python3 -m unittest scripts/test_cargo_license_state.py` — 9 tests passés.
- `python3 scripts/verify_cargo_license_state.py` — `verified`; `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`, marqueur personnalisé `NOASSERTION`.
- `cargo test --offline --locked --bin forgejo-reconcile-local private_resource_not_found_is_inconclusive_and_explains_visibility_ambiguity -- --nocapture` — 1 passé.
- `cargo test --offline --locked --bin forgejo-reconcile-local capabilities_list_exact_merge_target_profiles -- --nocapture` — 1 passé.
- `cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --nocapture` — 1 passé; l’API découverte reste `forgejo-reconcile-capabilities-v1` et le champ est une extension additive.
- `cargo build --offline --locked --bin forgejo-reconcile-local` — exit 0; SHA-256 du binaire `target/debug/forgejo-reconcile-local`: `080ff0eb66ea58ca10de97193c29178f6c7b06d7630c10efc30673b7596d5962`.
- `target/debug/forgejo-reconcile-local --capabilities` — exit 0; la sortie JSON a été relue et vérifiée contre le contrat attendu.
- `cargo fmt --all -- --check` et `git diff --check` — exit 0.
- `cargo metadata --locked --offline --format-version 1 --no-deps` — exit 0; les valeurs brutes et le résumé relu sont archivés dans `cargo-metadata.json` et `readback-summary.json`.

Versions : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, Python `3.12.3`. Les sorties ciblées sont conservées à côté de ce rapport.

## Fondement et portée

Le [Cargo Book, format du manifeste](https://doc.rust-lang.org/cargo/reference/manifest.html) sépare les champs `license`/`license-file` des métadonnées `package.metadata`, que Cargo ignore et réserve aux outils externes. Le [SPDX Specification 2.3.1, §7.15](https://spdx.github.io/spdx-spec/v2.3.1/package-information/#715-declared-license-field) autorise `NOASSERTION` au champ SPDX « Declared License » lorsque le créateur n’a pas déterminé ce champ ou ne fournit volontairement pas d’information; l’absence du champ a un sens équivalent. Notre clé demeure personnalisée et n’est pas le champ Cargo `license`.

Le 404 utilisé comme référence provient du précédent readback HTTP contre Forgejo 17.0.0 local; cette tranche n’a pas relancé l’instance et ne prétend pas que tous les Forgejo masquent les mêmes ressources. Le test Rust vérifie l’erreur et la sortie sans état canonique; le test de capacités vérifie le contrat compilé. Ce n’est pas une preuve de scope minimal du credential, de merge atomique, d’agent LLM, de benchmark ou de droits de redistribution. Aucun service externe, secret réel ou dépense nouvelle n’a été utilisé.

Manifest local : `SHA256SUMS`.
