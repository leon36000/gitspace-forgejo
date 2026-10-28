# CONTINUATION-343 — refus précoce d’une PR brouillon par l’autorité de fusion

Date : 28 octobre 2026. Worktree local : `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun changement Forgejo/GitLab distant.

## Changement

Le chemin `forgejo-reconcile-local --merge-local-test` vérifie maintenant le champ `draft` du readback API courant immédiatement après le décodage `PullRequestSnapshotV1`. Si la PR est en brouillon, il renvoie `verdict=rejected`, `error.code=draft_pull_request`, `retryable=false`, l’action de résoudre le brouillon par le processus normal de revue et `details.stage=pull_request_read`. Le refus intervient avant les lectures d’acteur, de revues, de protection de branche et de base.

Le test processus `draft_pull_request_refuses_merge_authority_before_review_or_post` fournit une réponse API courante avec `draft=true`. Il exige un code de sortie non nul, aucune autorisation/tentative de fusion, un seul GET de la PR, aucun autre GET, aucun POST et aucun intent de fusion durable.

## RED/GREEN et vérifications

RED, avant le changement : `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only draft_pull_request_refuses_merge_authority_before_review_or_post` échoue comme attendu; la réponse était `preflight_rejected` et n’exposait pas la cause. Sortie complète dans `red.log`.

GREEN, après le changement : le même runner passe 1/1, 0 échec, 9 scénarios filtrés, sous Bubblewrap 0.9.0 avec `--unshare-net --unshare-pid`; source montée en lecture seule, cible API limitée au fixture loopback. `green.log` contient le run final après formatage.

`cargo test --offline --locked --all-targets -- --test-threads=1` passe avec 730 réussis, 0 échec, 161 ignorés en 38 groupes. Le nouveau test processus est ignoré par défaut et a été lancé séparément par le runner; `tests/forgejo_merge_preflight.rs` passe 13/13 dans la suite.

`cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_forgejo_mission_delivery_process_e2e.py`, `python3 scripts/verify_cargo_license_state.py` et `python3 scripts/test_cargo_license_state.py` passent. Le test de la politique Cargo passe 12/12. `cargo metadata --offline --locked --no-deps --format-version 1` confirme `license=null`, `license_file=null`, `publish=[]`; aucune licence produit n’est choisie, les droits restent `not_assessed` et SPDX reste `NOASSERTION`. Sorties dans `verification.log`.

Environnement : Rust 1.95.0, Cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0, Linux aarch64.

Empreintes SHA-256 des sources finales : `src/bin/forgejo-reconcile-local.rs` 38edbee0b74a92137aa20f51ebd887aa6a4a2cf2fb6e8698b9835e6fe2adf11b; `tests/forgejo_merge_post_crash_recovery_process_e2e.rs` 96ab823763a3b4e80b44195608a30f21ec5462b9e0556150e5f8fe6ba435b848; `scripts/run_forgejo_mission_delivery_process_e2e.py` e836490f5e2c6d02efe6417c4a141d1dfe32e3989b1b50c33fd0d607d94743d8.

## Portée et suite

Ce run n’utilise qu’une fixture HTTP locale avec données et credential synthétiques. Il ne démontre pas le comportement d’une instance Forgejo live, une fusion, l’exclusion des autres writers, AgentOS/LLM ou une amélioration générale. Aucun coût, dépendance, accès ou service externe n’a été ajouté.

Le checkpoint et la matrice de refus référencent ce lot. Les manifestes append-only locaux, worktree et projet sont vérifiés dans `manifest-check.txt` et `global-manifest-check.txt`. Prochaine étape : choisir une lacune distincte encore ouverte dans la matrice, sans rejouer les refus brouillon déjà couverts.