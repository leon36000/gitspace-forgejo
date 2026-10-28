# WEBHOOK-404-BATCH-RECONCILE-001 — ressource supprimée dans un batch

Date : 25 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; il reste fortement modifié. Les modifications préexistantes ont été conservées.

## Résultat

Le nouveau scénario processus `actual_reconcile_cli_process_reports_deleted_resource_without_aborting_batch` fait admettre par le vrai binaire receiver trois webhooks signés avec des IDs distincts : deux visent la même issue 41 et le troisième vise l’issue 42. Le processus `forgejo-reconcile-local` relit ensuite l’inbox et consulte une fixture HTTP loopback privée. Celle-ci répond 404 pour l’issue 41, avec un corps hostile distinctif, puis renvoie l’état courant valide de l’issue 42.

Le batch reste `inconclusive`. Les deux livraisons de l’issue absente sont toutes deux `inconclusive`, code `resource_not_found`, `retryable=false`, sans état canonique. Le contenu du corps 404 n’apparaît pas dans la sortie. L’issue 42 reçoit néanmoins `current_state_read` et son état canonique `open`. Deux requêtes GET seulement sont observées : le 404 est terminal et la réponse d’erreur du premier GET est mise en cache pour l’autre livraison de l’issue 41. Aucun header Authorization n’est envoyé.

Le traitement testé correspond au mapping HTTP 404 → `ApiReadError::NotFound` dans `src/bin/forgejo-reconcile-local.rs`, au code d’erreur structuré `resource_not_found`, et au cache batch des résultats `Result<Value, ApiReadError>` dans `src/webhook/reconcile.rs`. Aucun changement de logique produit n’a été nécessaire; cette tranche ajoute le scénario de non-régression et son entrée au runner isolé.

## Vérifications

| Commande | Résultat |
|---|---|
| `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reports_deleted_resource_without_aborting_batch` | code 0; test processus 1/1, 23 autres cas filtrés; log `process-e2e.log` |
| `cargo fmt --all -- --check` | code 0; `rustfmt.log` |
| `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` | code 0; `python-compile.log` |
| `git diff --check` | code 0; `diff-check.log` |

Environnement : Rust/Cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0, Linux aarch64 (`versions.txt`). Les empreintes SHA-256 des sources de l’adaptateur, de la réconciliation, du scénario, du runner et du manifeste Cargo sont dans `source-sha256.txt`.

## Portée et limites

Le receiver et le CLI sont les processus réels du dépôt. L’API Forgejo est une fixture locale synthétique exécutée sous Bubblewrap avec le réseau isolé (`--unshare-net`); aucune instance Forgejo, credential réelle ou donnée externe n’est utilisée. Ce test ne prouve pas un parcours PR/vérification/fusion, l’usage d’un LLM ou d’AgentOS/RAG/mémoire, ni une amélioration mesurée. Il est ignoré par défaut dans Cargo et sélectionné explicitement par le runner dédié.

Aucune licence n’est sélectionnée par cette tranche. Aucune publication, modification de production, dépense, suppression ou extension d’accès n’a eu lieu. Le manifeste local est vérifié séparément par `sha256sum -c SHA256SUMS` depuis ce répertoire.
