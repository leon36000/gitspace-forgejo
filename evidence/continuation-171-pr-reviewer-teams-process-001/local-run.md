# CONTINUATION-171 — projection et refus au processus

Date : 26 octobre 2026. Worktree courant : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre sale avec du travail antérieur conservé. Ce lot ajoute deux scénarios processus ignorés par défaut à l’all-targets Rust et les autorise pour exécution explicite dans le runner Bubblewrap. Il ne modifie pas le code de production ni `Cargo.toml`.

## Parcours testé

Chaque scénario démarre le vrai `forgejo-webhook-local`, envoie un événement `pull_request` signé avec la clé synthétique du test, confirme l’acceptation et l’arrêt du receiver, puis exécute le vrai `forgejo-reconcile-local` contre un serveur HTTP fixture loopback.

1. `actual_reconcile_cli_process_projects_current_pull_request_review_teams` sert un readback courant de PR contenant un reviewer utilisateur et deux équipes avec des métadonnées supplémentaires. Le CLI retourne le schéma single 7, une lecture amont, les reviewers utilisateur sans e-mail, les équipes `{id,name}` triées, et aucun champ de description/organisation/unités. Une seule requête GET est observée; elle ne contient pas `Authorization`.
2. `actual_reconcile_cli_process_refuses_duplicate_pull_request_review_team_ids` sert deux équipes avec le même ID. Le CLI termine code 1 et expose `inconclusive`, `invalid_current_state`, `retryable=false` et `details.field=requested_reviewers_teams`; il ne produit pas de `canonical_state`. La requête est un seul GET sans `Authorization`; le journal d’événements reste identique octet pour octet.

Les deux scénarios passent individuellement `1 passed; 0 failed; 0 ignored` (127 autres tests filtrés). Ils ont été exécutés après formatage final par :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_projects_current_pull_request_review_teams
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_duplicate_pull_request_review_team_ids
```

Les commandes sont reproduisibles sans Forgejo, API externe, credential Forgejo ou modèle. Le runner copie le source dans un répertoire staged, compile offline/locked avec Rust 1.95.0 et exécute chaque cas sous Bubblewrap 0.9.0 avec `--unshare-net`, `--unshare-pid`, environnement effacé, arbre source read-only et `/tmp` isolé. Le test utilise une API loopback déterministe. Logs complets : [`isolated-process-test.log`](isolated-process-test.log), [`malformed-refusal-process-test.log`](malformed-refusal-process-test.log).

## Licence Cargo

Readback frais : `cargo metadata --offline --locked --no-deps --format-version 1` rapporte `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]` et les métadonnées `decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`. `python3 scripts/verify_cargo_license_state.py` passe. Le SHA-256 de `Cargo.toml` demeure `f9144c83c85369f40ecfd2255fdd0915ddb90a4d6c14e1705908febe9bb2b1a3`, identique au reçu CONTINUATION-170; le SHA-256 de `Cargo.lock` est `bf599559dd11ff21ecc4754aa8a693966b4a9ca401153d7d7caa1f18d8995fcc`. Aucun champ Cargo `license`/`license-file` n’est ajouté et aucun droit de redistribution n’est déduit. Voir [`cargo-metadata-summary.json`](cargo-metadata-summary.json) et [`cargo-license-guard.json`](cargo-license-guard.json).

## Vérifications locales

- Deux tests processus ciblés, exécutés séparément sous le runner Bubblewrap : 1/1 chacun.
- `cargo fmt --all -- --check` : passe après correction de trois changements de wrapping trouvés par le premier check; cette sortie initiale est conservée comme [`rustfmt-initial-diff.log`](rustfmt-initial-diff.log).
- `git diff --check` : passe.
- `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` : passe.
- `python3 scripts/verify_cargo_license_state.py` : passe.
- `cargo metadata --offline --locked --no-deps --format-version 1` : readback ci-dessus.

Le total `cargo test --offline --locked --all-targets -- --test-threads=1` de CONTINUATION-170 (647 passés, 0 échec, 115 ignorés sur 36 blocs) n’a pas été relancé. Les deux nouveaux cas sont ignorés par ce chemin et ont été activés par la commande dédiée; la mission n’assimile pas les tests ignorés au passage du cas. Toolchain : Rust/Cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0, hôte `aarch64-unknown-linux-gnu`.

## Portée et suite

Le DTO du GET est un fixture synthétique loopback; aucune instance Forgejo live n’a été démarrée et aucun appel à `/requested_reviewers` ou `team_reviewers` n’a été fait. Les tests démontrent le comportement du vrai receiver, du journal et du CLI face à des formes d’API précises, pas la production Forgejo, les permissions d’organisation, l’atomicité des GET, un agent LLM/AgentOS, un merge ou un gain comparatif. Le code Forgejo n’est ni copié ni modifié; GitSpace ne participe pas à ce lot.

Prochaine action : choisir une lacune cumulative distincte qui admet une preuve locale sans coût ni accès nouveau. Les critères du parcours LLM avec runtime autorisé, la vérification post-merge courante, l’autorité contre tous writers, les droits/licences et le benchmark apparié restent ouverts.
