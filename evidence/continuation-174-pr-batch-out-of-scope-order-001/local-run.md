# CONTINUATION-174 — garder le résultat autorisé dans les deux ordres du batch

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre déjà fortement modifié et préservé. La tranche étend uniquement un test processus existant et son allowlist; aucun code de production, dépendance, manifeste Cargo ou lockfile n’a changé.

## Parcours exécuté

Le test `actual_reconcile_cli_process_preserves_configured_same_number_pull_request_in_both_batch_orders` fait accepter au vrai receiver Rust deux deliveries signées, toutes deux en HTTP 202 et séquences 1/2 : une PR #23 de `forge-admin/agent-forge-core` et une PR #23 de `forge-admin/agent-forge-sibling`. Le journal durable est pris en snapshot.

Le vrai `forgejo-reconcile-local` reçoit les mêmes IDs deux fois, en deux invocations séquentielles et inversées :

1. dépôt configuré puis dépôt frère;
2. dépôt frère puis dépôt configuré.

Le serveur de fixture sert une réponse PR valide à chaque invocation. Dans les deux ordres, le CLI n’émet qu’un GET vers `/repos/forge-admin/agent-forge-core/pulls/23`; le dépôt frère ne reçoit aucune requête. Le lot se termine `inconclusive` parce qu’il contient un refus, mais la delivery configurée reste `current_state_read`, avec le head, la base et les reviewers du DTO. La delivery étrangère reste `resource_out_of_scope`, `retryable=false`, sans `canonical_state` ni `details.resource`; son action demande le routage vers l’adaptateur correspondant et interdit le retry sur l’instance courante. Les résultats restent dans l’ordre de chaque invocation. Aucune requête n’a de header `Authorization` et le journal demeure byte-for-byte inchangé.

Commande exacte :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_preserves_configured_same_number_pull_request_in_both_batch_orders
```

Résultat ciblé source-courant : `1 passed; 0 failed; 0 ignored; 129 filtered out`. Sortie finale : [`process-test-final.log`](process-test-final.log); le premier run de la version avec les deux permutations est aussi conservé dans [`process-test.log`](process-test.log). L’exécution utilise Bubblewrap 0.9.0 avec namespace réseau/PID non partagés, environnement vidé, source staged read-only et `/tmp` isolé. Les deux réponses HTTP sont des fixtures distinctes; le test ne lance pas Forgejo.

## Vérifications et métadonnées

`cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, `python3 scripts/verify_cargo_license_state.py` et le readback `cargo metadata --offline --locked --no-deps --format-version 1` passent. Empreintes des sources : [`source-hashes.txt`](source-hashes.txt); versions : [`environment.txt`](environment.txt).

Cargo reste `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, avec le champ personnalisé `spdx_package_license_declared=NOASSERTION`. Aucune licence ou permission de redistribution n’est choisie ou déduite. Aucun test Rust all-targets n’est relancé.

## Limites

Les permutations sont séquentielles et réutilisent le même journal; aucune mutation concurrente ni panne/crash n’est simulé. L’API est une fixture loopback, pas Forgejo live. Le CLI reste configuré pour un seul dépôt; aucun routeur multi-dépôts, LLM, AgentOS, Forge Code, credential de fusion ou benchmark n’est impliqué. La preuve confirme le comportement du client local face à l’ordre du batch, pas les ACL du serveur ou l’intégrité générale de production. Le projet demeure incomplet.
