# WEBHOOK-PR-BASE-SHA-ACTIONABILITY-001 — résultat local

Date : 2026-10-25. Dépôt de travail : `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree est fortement modifié et comprend des fichiers non suivis; ce SHA identifie la base, tandis que `source-sha256.txt` identifie les sources de travail effectivement vérifiées.

## Résultat

Le test `actual_reconcile_cli_process_reports_invalid_pull_request_base_sha_field` fait passer une livraison `pull_request` synthétique signée au véritable processus récepteur, vérifie son acceptation (202), puis exécute `forgejo-reconcile-local` contre une fixture HTTP loopback. Celle-ci renvoie HTTP 200 avec une PR ouverte, un SHA `head.sha` valide et `base.sha="not-a-commit-sha"`.

Le CLI termine en échec avec un JSON structuré version 2 : `verdict=inconclusive`, `error.code=invalid_current_state`, `retryable=false`, `details.field=base.sha`, `details.resource.kind=pull_request` et `details.resource.number=25`. Il n’émet pas `canonical_state`. Le test observe un unique `GET /api/v1/repos/forge-admin/agent-forge-core/pulls/25`, aucun en-tête `Authorization`, et un journal durable identique avant/après réconciliation. L’erreur donne un champ et une ressource précis; elle ne demande pas de réessayer automatiquement.

Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reports_invalid_pull_request_base_sha_field` — 1/1, code 0. Le runner compile les cibles et exécute ce test Cargo ignoré dans Bubblewrap 0.9.0 avec `--unshare-net`; seule la fixture loopback du scénario est permise. Sortie complète : `invalid-pr-base-sha.log`; code : `invalid-pr-base-sha.exit`.

Vérifications connexes fraîches : `python3 scripts/test_cargo_license_state.py` — 7/7; `python3 scripts/verify_cargo_license_state.py` — `verified`; `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`; `cargo fmt --all -- --check`; `git diff --check` — tous code 0. Chaque commande a un fichier `.log` et `.exit` correspondant. Le premier `git diff --check` après l’ajout du rapport a signalé une ligne blanche supplémentaire en fin de `docs/implementation-plan.md` (code 2); elle a été retirée, puis `git diff --check` a été relancé et passe (code 0). Les deux résultats sont conservés. Les versions d’outils sont dans `tool-versions.txt`.

## Changement et limites

Ce lot ajoute le scénario processus et l’enregistre dans le runner ciblé; la validation de `base.sha` et le refus structuré existaient déjà dans l’implémentation. Aucun code de production ni code Forgejo n’a changé. Ce résultat est une preuve de processus contre une fixture HTTP locale, pas une observation d’une instance Forgejo réelle. Il n’exerce pas de fusion, d’agent LLM, d’AgentOS ou de valeur utilisateur; aucun benchmark comparatif n’est effectué. Les parcours de commentaire, reprise, désordre et interruption déjà prouvés ne sont pas rejoués ici.

Le garde Cargo lit `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`; aucune licence produit n’est choisie et les droits des dépendances ne sont pas évalués par ce contrôle.

