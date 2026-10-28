# WEBHOOK-PR-APPROVED-REVIEW-RECONCILE-001 — résultat local

Date : 2026-10-25. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Worktree dirty avec modifications antérieures préservées; les empreintes du rapport fixent les sources de travail.

## Parcours vérifié

Le test `actual_reconcile_cli_process_routes_approved_review_to_pull_request_api` envoie au processus récepteur réel une livraison Forgejo v17 synthétique, signée sur les octets bruts : événement `pull_request_approved`, action `reviewed`, PR #26 et `review.type=pull_request_review_approved`. Le récepteur renvoie 202, indique `accepted=true`, `duplicate=false` et `reconcile_required=true`. Le CLI réel lit cette livraison depuis le journal durable.

Une fixture HTTP loopback privée répond 200 à un seul `GET /api/v1/repos/forge-admin/agent-forge-core/pulls/26`. Le client n’envoie pas de header `Authorization`. La notification déclare `open`, `merged=false` avec SHA `a…`; l’API fixture déclare `closed`, `merged=true` avec SHA `1…` et `2…`. Le résultat structuré JSON v2 est `current_state_read`, ressource `pull_request` #26, et reprend l’état et les SHA de l’API. Le journal reste identique octet pour octet après le readback.

Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_routes_approved_review_to_pull_request_api` — 1/1, code 0, exécutée sous Bubblewrap 0.9.0 avec `--unshare-net`. Le log du run final est `approved-review-process-e2e.log`; code dans `.exit`.

Vérifications fraîches : `cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, `python3 scripts/test_cargo_license_state.py` (7/7), `python3 scripts/verify_cargo_license_state.py` (`verified`) — tous code 0 après correction du rapport. Le premier `git diff --check` a signalé une ligne blanche supplémentaire en fin de `docs/implementation-plan.md` (code 2); elle a été retirée puis le contrôle a été relancé. Les deux résultats sont conservés. Les autres logs et codes figurent dans le dossier. Versions dans `tool-versions.txt`.

## Limites

C’est une composition locale avec binaires réels du dépôt et une fixture HTTP; ce n’est pas un test contre une instance Forgejo réelle. Il n’y a pas d’authentification, de décision utilisateur sur l’approbation, de test de permission du reviewer, de vérification ou de fusion. Aucun modèle/AgentOS n’est utilisé et aucun gain comparatif n’est mesuré. Le code de production n’a pas changé dans cette tranche.

Le garde Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`; aucune licence produit n’est choisie et les droits des dépendances ne sont pas établis.
