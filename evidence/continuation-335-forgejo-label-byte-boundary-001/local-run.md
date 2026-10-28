# CONTINUATION-335 — borne UTF-8 cohérente pour les labels

Date : 28 octobre 2026. Révision de base du worktree : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Cette intervention locale ne modifie pas le remote Forgejo ou GitLab.

## Défaut reproduit

La mission CLI rejetait un label dont le nom dépasse 255 octets UTF-8, tandis que `current_work_item_labels` dans `src/webhook/reconcile.rs` vérifiait 255 points de code Unicode. Un nom de 128 caractères `é` vaut 256 octets : la réconciliation le publiait comme état canonique, puis l’admission mission le refusait.

Le test first `current_api_label_name_limit_matches_runtime_utf8_byte_limit` a échoué avant correction (`red.log`, code de sortie 101) : résultat observé `Ok(ReconciledState { labels: Some(...) })`, attendu `Err(InvalidCurrentStateField("labels"))`.

## Changement

- `src/forgejo.rs` définit `MAX_FORGEJO_LABEL_NAME_BYTES = 255`.
- `MAX_UNTRUSTED_WORK_ITEM_LABEL_NAME_BYTES` et `MAX_RECONCILED_LABEL_NAME_BYTES` réutilisent cette même constante.
- La réconciliation vérifie désormais `String::len()`; le nom à 255 octets reste valide et celui à 256 octets est inconclusif/refusé. `MAX_RECONCILED_LABEL_NAME_CHARS` est conservé en alias déprécié pour compatibilité source.
- Le contrat CLI des capacités est versionné v22; la projection des labels expose `maximum_name_bytes`. La borne `maximum_name_characters` des équipes de réviseurs n’a pas changé.
- Le test unitaire de projection du CLI mission contient aussi un nom multioctet de 256 octets dans ses cas mal formés.

## Vérifications locales

- Rouge initial : `cargo test --offline --locked --test webhook_reconcile_contract current_api_label_name_limit_matches_runtime_utf8_byte_limit -- --exact --nocapture` — 0 réussi, 1 échec intentionnel avant correction (`red.log`).
- Vert frontière : même sélection après correction — 1 réussi (`green.log`). Elle couvre 255 octets valides et 256 octets refusés.
- `cargo test --offline --locked --test webhook_reconcile_contract --test forgejo_reconcile_capabilities_e2e -- --test-threads=1` — 85 tests de réconciliation et 5 tests de capacités réussis, aucun échec; 2 tests de capacités ignorés parce qu’ils nécessitent le port fixe 3130 (`contract-tests.log`).
- `cargo test --offline --locked --bin forgejo-mission-local issue_label_projection_distinguishes_unknown_empty_and_malformed -- --nocapture` — 1 réussi, incluant le cas multioctet (`runtime-label-test.log`).
- `cargo fmt --all -- --check` et `git diff --check` — réussite (`rustfmt.log`, `diff-check.log`).
- `python3 scripts/verify_cargo_license_state.py` — réussite; `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution `not_assessed`, `NOASSERTION` (`cargo-license-state.log`).
- Une relance finale après modification du test admission et des documents regroupe ces vérifications et contrôle à nouveau tous les hashes sources (`final-checks.log`, tous les codes retour à 0).

Les empreintes des sources vérifiées sont dans `source-hashes.txt`; la somme locale et les indices append-only se trouvent dans `SHA256SUMS` après génération.

## Portée et limites

Les tests sont des contrats Rust locaux avec API fixture et le binaire local de capacités; le test d’admission est unitaire. Aucun Forgejo distant ou GitLab n’a été contacté, aucun agent LLM/AgentOS n’a été exécuté, et aucun PR, merge, vérificateur indépendant ou comportement post-fusion n’a été observé. Ce correctif élimine une incompatibilité de limite; il ne démontre pas un gain global ni la complétion de la forge.
