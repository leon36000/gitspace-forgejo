# WEBHOOK-PR-REVIEW-COMMENT-RECONCILE-001 — résultat local

Date : 25 octobre 2026. Worktree : local/WEBHOOK-005-canonical-links, HEAD de base f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0. Le dépôt produit était déjà fortement modifié; ce lot n’ajoute qu’un test processus et son entrée dans le runner. Les sources correspondantes sont identifiées par hashes dans source-sha256.txt.

## Parcours vérifié

Le test actual_reconcile_cli_process_routes_pull_request_review_comment_to_pull_request_api envoie au récepteur réel un corps Forgejo v17 synthétique signé sur les octets bruts. L’événement HTTP est pull_request_comment et le corps contient review.type=pull_request_review_comment, la PR #29 et un commentaire de revue. Le récepteur répond 202 avec accepted=true, duplicate=false et reconcile_required=true; la livraison apparaît dans l’inbox durable.

Le vrai CLI de réconciliation fait un seul GET /api/v1/repos/forge-admin/agent-forge-core/pulls/29 à une fixture HTTP loopback privée. Il n’envoie aucun header Authorization. Le webhook annonce open/merged=false avec les SHA initiaux; la fixture API répond closed/merged=true avec des SHA différents. La sortie JSON v2 est current_state_read, ressource pull_request #29, et utilise l’instantané API courant. L’inbox reste identique octet pour octet après le readback.

Commande : python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_routes_pull_request_review_comment_to_pull_request_api — 1/1, code 0, sous Bubblewrap avec --unshare-net. La compilation hors réseau est verrouillée sur Cargo.lock. La sortie brute et le code sont dans review-comment-process-e2e.log et review-comment-process-e2e.exit.

## Vérifications et portée

Les vérifications finales comprennent cargo fmt --all -- --check, git diff --check, python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py, python3 scripts/test_cargo_license_state.py (7/7) et python3 scripts/verify_cargo_license_state.py (verified). Les sorties sont conservées dans ce dossier. Le garde laisse license=null, license_file=null, publish=[], license_decision=no_license_selected et redistribution_rights=not_assessed; aucune licence n’est choisie.

Cette preuve compose le vrai récepteur, le journal et le CLI contre une fixture HTTP locale; aucun serveur Forgejo réel, agent, LLM, vérificateur de code ou merge n’est impliqué. Elle teste le routage de commentaire de revue distinct du commentaire de conversation PR (issue_comment), pas la justesse de la revue ni les permissions de son auteur. Aucun code de production n’a changé.
