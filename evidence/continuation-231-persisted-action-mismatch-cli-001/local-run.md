# CONTINUATION-231 — refus CLI au replay d’une action workflow incohérente

Date : 2026-10-27. Worktree `webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dépôt dirty.

## Résultat

Le scénario accepte d’abord via le vrai receiver un webhook signé `workflow_run_blocked` avec `action=run_status_changed`. La fixture remplace ensuite l’action persistée par `run_completed` et recalcule le hash de ligne; le replay du journal confirme que la chaîne de hash reste valide. Le CLI de réconciliation relit l’événement et refuse avant tout accès amont avec `verdict=inconclusive`, `error.code=invalid_envelope`, `details.field_path=payload.action`, `persistence_state=already_persisted`, `source_hmac_persisted=false`, `source_hmac_reverified=false` et zéro lecture upstream. Un listener réservé à l’origine Forgejo ne reçoit aucune connexion; les octets du journal après le refus sont identiques à ceux d’avant le CLI.

Commande :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_persisted_incompatible_workflow_action_before_api_read
```

Le lanceur compile hors réseau puis exécute le test ignoré dans Bubblewrap avec `--unshare-net --unshare-pid`; le résultat est 1/1, 140 tests filtrés. La sortie complète est dans `isolated-cli-process.log`. `cargo fmt --all -- --check` et `git diff --check` passent.

## Portée

Cette preuve complète les refus à l’ingestion du receiver (CONT-230) et la table des 14 couples (CONT-229) en exerçant le chemin réel CLI → inbox persistée → validation avant réseau. La ligne incohérente est synthétique et n’a pas été signée sous cette forme; le test ne prétend pas que son hash de chaîne atteste l’origine, le HMAC ou le contenu. Le journal ne garde pas le HMAC original, limite rapportée par le CLI.

Aucun serveur ou runner Forgejo, LLM, AgentOS, RAG ou mémoire n’est utilisé. L’état Cargo et l’absence d’adaptateur modèle ont été relus et documentés dans [CONTINUATION-230](../continuation-230-workflow-action-process-refusal-001/local-run.md); aucune licence n’est sélectionnée et aucun service externe n’est appelé. Cette preuve de refus ne couvre pas toutes les routes ou versions Forgejo et ne démontre aucun gain comparatif. Le projet reste incomplet.
