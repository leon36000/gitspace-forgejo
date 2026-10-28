# WEBHOOK-PR-REJECTED-REVIEW-RECONCILE-001 — résultat local

Date : 25 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. L’arbre était déjà fortement modifié; ce lot conserve ces changements et n’ajoute qu’un test processus et son entrée dans le runner. Les hashes des sources du lot sont archivés dans `source-sha256.txt`.

## Parcours vérifié

Le test `actual_reconcile_cli_process_routes_rejected_review_to_pull_request_api` envoie au processus récepteur réel une livraison Forgejo v17 synthétique, signée sur ses octets bruts : événement `pull_request_rejected`, action `reviewed`, PR #28 et `review.type=pull_request_review_rejected`. Le récepteur renvoie 202, `accepted=true`, `duplicate=false` et `reconcile_required=true`. Le vrai CLI de réconciliation lit ensuite cette livraison dans le journal durable.

La fixture HTTP loopback privée répond 200 à un seul `GET /api/v1/repos/forge-admin/agent-forge-core/pulls/28`. Le client n’envoie pas de header `Authorization`. La notification indique `open/merged=false` et des SHA de tête/base initiaux; l’API fixture fournit l’état courant `open/merged=false` avec des SHA distincts. La sortie JSON v2 est `current_state_read`, ressource `pull_request` #28, et reprend les SHA de l’API. L’inbox reste identique octet pour octet après la réconciliation.

Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_routes_rejected_review_to_pull_request_api` — 1/1, code 0, sous Bubblewrap avec `--unshare-net`. La compilation Cargo offline/locked et la sortie finale du scénario figurent dans `rejected-review-process-e2e.log`; le code de sortie est dans `rejected-review-process-e2e.exit`.

Vérifications après documentation : `cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, `python3 scripts/test_cargo_license_state.py` (7/7) et `python3 scripts/verify_cargo_license_state.py` (`verified`). Les neuf empreintes source passent; le manifeste local couvre 15 fichiers et passe 15/15. Les 20 dernières entrées globales pour les sources et cette preuve correspondent à leurs fichiers courants. L’historique global reste append-only et comporte d’anciennes divergences; elles ne sont pas présentées comme validées. Les sorties et codes sont conservés dans ce dossier.

## Limites

C’est une composition du récepteur, du journal et du CLI avec une fixture HTTP locale; aucune instance Forgejo réelle n’a été exercée. Le test ne valide pas l’effet métier d’un rejet, l’identité ou les permissions de l’auteur de revue, une vérification de code, une fusion, un agent LLM, ni un avantage comparatif. Aucun code de production n’a changé.

Le garde Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`; aucune licence produit n’est choisie et les droits des dépendances restent non établis.
