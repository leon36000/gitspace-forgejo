# WEBHOOK-OUT-OF-ORDER-CURRENT-STATE-001 — le journal conserve l’ordre d’arrivée, l’état vient de Forgejo

Date : 25 octobre 2026. Worktree produit au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, fortement dirty. Aucun commit n’a été créé.

## Contrat examiné

Deux notifications signées concernent la même issue. La plus récente (`closed`, 14:00 UTC) est reçue en premier; une notification plus ancienne (`open`, 13:00 UTC) arrive ensuite avec un nouvel identifiant de livraison. Le rejeu exact de la première livraison doit être reconnu comme doublon, tandis que le nouvel identifiant ancien reste auditable dans le journal. La réconciliation doit lire l’état canonique courant du Forgejo configuré, préserver les IDs et ne réécrire ni trier le journal.

Le test ajouté `late_stale_notification_is_reconciled_to_current_forgejo_state` traverse la validation HTTP/HMAC, `handle_webhook_request`, le journal durable, son replay et `reconcile_inbox_events`. Il constate deux enregistrements dans l’ordre d’arrivée, un seul snapshot canonique fermé avec date 14:00 pour les deux résultats, et un seul appel à l’interface de lecture courante dans le batch.

Le scénario de processus préexistant `actual_reconcile_cli_process_uses_current_state_after_out_of_order_webhooks` a aussi été lancé via son runner dédié. Il démarre le vrai récepteur puis la vraie CLI de réconciliation, redonne les livraisons dans l’ordre ancien-après-récent, lit une fixture HTTP loopback qui retourne l’état fermé courant, observe un seul GET, reçoit deux résultats liés aux bons IDs et vérifie que le journal reste octet pour octet identique. Test : `tests/webhook_receiver_process_e2e.rs` autour de la ligne 1342; runner : `scripts/run_webhook_reconcile_cli_e2e.py`.

## Résultats reproductibles

- Test de contrat ciblé : 1 réussi, 0 échec; journal `focused.log`.
- Processus isolé : 1 réussi, 0 échec, 28 filtrés. Commande :

  ```sh
  python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_uses_current_state_after_out_of_order_webhooks
  ```

- Runner processus : Bubblewrap 0.9.0, `--unshare-net` et `--unshare-pid`, source montée en lecture seule, environnement nettoyé; la seule API est une fixture loopback. Journal `cli-process-e2e.log`.
- Suite Rust complète après changement : `CARGO_TARGET_DIR=/tmp/agent-forge-webhook-045-target cargo test --offline --locked --all-targets -- --test-threads=1` : 468 réussis, 0 échec, 33 ignorés, 34 suites de résultats; journal `all-targets.log`.
- `rustfmt --edition 2024 --check tests/webhook_reconcile_pipeline.rs` et `git diff --check -- tests/webhook_reconcile_pipeline.rs` passent.

Versions et empreintes des sources utilisées : `tool-versions.txt` et `source-hashes.txt`. Le manifeste local couvre les journaux, le rapport, les métadonnées et les vérifications.

## Portée et limites

Le processus CLI et le récepteur sont réels, mais Forgejo est simulé par une fixture HTTP loopback; aucune instance Forgejo live n’a émis les événements. Le test nouvellement ajouté est test-only : aucune logique de production n’a changé. Le fichier de test était non suivi dans ce worktree dirty; aucun commit ni stage n’a été créé. Ce résultat confirme l’idempotence par ID de livraison et l’autorité de l’état relu pour ce cas, pas la sémantique complète de tous les types d’événements, un gain de productivité, un parcours LLM ou la comparaison Forgejo + agent + CI conventionnels. Aucun AgentOS, LLM, service externe, coût ou secret n’a été utilisé.

La suite complète indique encore 33 tests ignorés, dont les autres scénarios de processus qui exigent le runner isolé; ils ne sont pas couverts par cette seule exécution ciblée. Les critères de refus cumulés, la validation live Forgejo et la comparaison contrôlée restent ouverts.
