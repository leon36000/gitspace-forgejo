# CONTINUATION-407 — `release.deleted` reste inconclusif après HTTP 404

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`; HEAD de base observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Scénario et résultat

Le scénario processus `actual_reconcile_cli_process_keeps_deleted_release_404_inconclusive_without_projection` utilise le receiver local réel pour admettre un webhook `release.deleted` signé sous une clé de test et inscrit à la séquence 1. Il lance ensuite le vrai binaire `forgejo-reconcile-local` contre un serveur HTTP loopback qui retourne 404 au GET exact `/api/v1/repos/{owner}/{repo}/releases/611`.

Le CLI quitte avec un code non nul et une réponse structurée `verdict=inconclusive`, `error.code=resource_not_found`, `retryable=false`. Les assertions lient la ressource au kind Release, propriétaire, dépôt et ID 611; elles valident l’attestation locale limitée à `receiver_signature_only`, l’absence d’état canonique, une seule tentative de readback, l’absence d’Authorization sur le GET public, l’absence des marqueurs privés de l’événement et de la réponse HTTP, et l’identité octet pour octet du journal avant et après réconciliation. La fixture n’observe qu’une requête GET sur la route exacte.

Commande d’exécution : `run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_keeps_deleted_release_404_inconclusive_without_projection`. Le helper Python a redirigé son répertoire temporaire vers `/dev/shm` pour éviter la partition racine pleine; Cargo a compilé hors ligne et Bubblewrap a isolé réseau et PID. Résultat : **1 passed, 0 failed, 194 filtered out**, code de sortie 0. La sortie complète est `process-test.log`.

## Vérifications et portée

Seul un test processus et l’entrée de scénario du runner sont concernés; aucune logique runtime n’a été modifiée. `cargo fmt --all -- --check`, `git diff --check` ciblé, l’analyse AST Python et la vérification de politique Cargo sont enregistrés dans ce dossier. Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, SPDX `NOASSERTION`.

Ce résultat prouve le comportement du CLI face à la réponse de fixture, pas l’état d’une instance Forgejo. HTTP 404 reste ambigu entre suppression et absence de visibilité; le verdict n’en infère pas une suppression réussie. Aucun LLM/AgentOS, pull request, décision de fusion, merge, accès GitLab authentifié ou benchmark n’est exercé. Le transfert GitLab demandé est toujours arrêté avant HTTP par la résolution/connexion PC6→PC4; aucun secret ni changement distant n’a été utilisé dans ce lot.
