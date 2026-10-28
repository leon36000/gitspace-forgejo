# CONTINUATION-087 — refuser un ID de run API différent au processus

Date : 26 octobre 2026. Produit : worktree /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace, branche local/WEBHOOK-005-canonical-links, HEAD de base f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0. Le worktree était déjà fortement modifié. Ce lot ajoute un test processus et met à jour le checkpoint/la matrice; aucun code runtime ni manifeste Cargo n’a changé.

## Scénario et résultat

Le receiver reçoit une enveloppe HMAC cohérente pour workflow_run_success : l’événement lie run_id=7001 et index_in_repo=41. Le receiver répond HTTP 202. Au readback, la fixture HTTP loopback répond avec un run ayant id=7002, mais conserve index, dépôt, statut, SHA, date et URL de forme valide.

Le vrai CLI forgejo-reconcile-local retourne inconclusive/resource_mismatch, sans canonical_state. Il n’effectue qu’un GET /actions/runs/7001, ne tente aucune lecture de job ou de logs, et laisse le journal append-only inchangé. Test : actual_reconcile_cli_process_refuses_workflow_run_api_id_mismatch. Le driver isole le processus avec Bubblewrap, namespaces réseau et PID, environnement effacé et source en lecture seule; voir process-e2e.log.

## Vérifications

- python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_workflow_run_api_id_mismatch : 1 réussi, 0 échec, 100 filtrés.
- cargo test --offline --locked --all-targets -- --test-threads=1 --quiet : 34 exécutables, 597 réussis, 0 échec, 93 ignorés (all-targets.log, test-summary.txt). Le cas processus est ignoré dans la suite générale et exécuté séparément par le driver isolé.
- cargo fmt --all -- --check, git diff --check et python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py passent.

L’état Cargo non choisi a été vérifié séparément dans [CONTINUATION-086](../continuation-086-source-inventory-reconcile-001/cargo-license-verify.json); ce lot n’a modifié ni le manifeste ni le lockfile.

## Limites

Le test traverse le receiver et le CLI réels, mais Forgejo est remplacé par une fixture HTTP loopback; il n’observe pas l’émission native d’événement ou un runner Actions. Il vérifie un ID de run divergent, pas toutes les corruptions possibles du readback. Aucun LLM, AgentOS, RAG, mémoire, coût, secret externe, publication ou production n’a été impliqué. L’identité du job/workflow reste consultative et n’est jamais une preuve d’acceptation ou d’autorisation de fusion.
