# CONTINUATION-412 — reprise du readback Release après HTTP 503

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`; HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours vérifié

Le test `actual_reconcile_cli_process_recovers_release_read_after_transient_503` traverse le vrai receiver et le vrai binaire `forgejo-reconcile-local`. Le receiver accepte une livraison `release.deleted` signée. La fixture API reçoit ensuite deux requêtes GET vers `/api/v1/repos/{owner}/{repo}/releases/615` et répond 503 à la première, 200 à la seconde.

Le CLI termine avec succès après **2 tentatives** et ne projette l’état qu’après le 200 : `verdict=current_state_read`, Release `present`, tag courant `v1.4.5`, source `current_forgejo_resource_api`, fraîcheur `current_api_read`. Les deux requêtes n’ont pas de header Authorization. Les contenus synthétiques de l’événement, du 503, du 200, des notes et assets sont absents de stdout/stderr. Le journal signé reste octet-identique après lecture.

Ce test ferme un cas manquant à la matrice : le retry du chemin Release dans le vrai CLI après erreur upstream transitoire, avec succès ultérieur. Les retries 503 génériques étaient déjà couverts sur d’autres ressources; cette preuve ne les répète pas comme test de Release.

## Commande et résultat

Le runner `scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_recovers_release_read_after_transient_503` compile avec Cargo offline/locked puis exécute le test sous Bubblewrap `--unshare-net --unshare-pid`. La commande exacte est dans `commands.txt`; le log complet est dans `process-test.log`.

Résultat : **1 passed, 0 failed, 0 ignored, 198 filtered out**, code de sortie 0. `cargo fmt --all -- --check`, `git diff --check`, l’analyse AST Python du runner, la garde de licence et `cargo metadata --offline --locked --no-deps --format-version 1` passent. La preuve Cargo garde `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`.

## Portée et limites

Le code modifié se limite au test processus et à sa liste dans le runner; aucune logique runtime, dépendance, configuration Cargo ou licence ne change. Les deux statuts HTTP viennent d’une fixture loopback synthétique. Cela prouve seulement le comportement du client local face à 503 puis 200; aucun Forgejo live, crash/reprise après redémarrage, AgentOS/LLM, PR/fusion ou benchmark n’est exercé.
