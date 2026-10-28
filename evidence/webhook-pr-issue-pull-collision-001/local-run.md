# WEBHOOK-PR-ISSUE-PULL-COLLISION-001 — preuve locale

Date : 2026-10-25. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Worktree sale; les sources ont des SHA-256 explicites dans ce dossier.

## Parcours

`actual_reconcile_cli_process_keeps_issue_and_pull_request_number_collision_separate` envoie deux livraisons HMAC valides au processus récepteur réel : `issues` #27 et `pull_request` #27. Le CLI lit leurs deux IDs depuis le journal durable et exécute un lot. La fixture API loopback renvoie pour l’issue `open` et pour la PR `closed/merged=true`, avec URLs et SHA correspondant à chaque type.

Le run final observe exactement deux GET : `/api/v1/repos/forge-admin/agent-forge-core/issues/27`, puis `/api/v1/repos/forge-admin/agent-forge-core/pulls/27`. Aucun Authorization. La sortie batch v3 a deux résultats et deux appels de lecture; elle conserve les delivery IDs, `resource.kind` (`issue` ou `pull_request`) et l’état courant correspondant à la réponse API de chaque route. Le SHA PR est conservé seulement dans la ligne PR; l’issue ne reçoit pas de données PR. Le journal ne change pas.

Commandes processus ciblées :
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_keeps_issue_and_pull_request_number_collision_separate` — 1/1, code 0.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_receiver_journal_against_private_http_fixture` — 1/1, code 0; revalidation de la sortie batch historique après correction de son assertion de version.

Le premier run du nouveau scénario a échoué uniquement sur l’assertion de test `schema_version`: le lot renvoie 3 et le test demandait 2. La sortie conservée (`same-number-issue-pr-process-e2e.log`) montre l’attente fautive; l’assertion est passée à 3 et le nouveau test passe. L’ancien test batch avait également une attente v2 incompatible; son assertion est passée à 3 puis ce scénario a été rejoué et passe. Aucun défaut du binaire n’a été observé.

Vérifications : `cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, `python3 scripts/test_cargo_license_state.py` (7/7), `python3 scripts/verify_cargo_license_state.py` (`verified`) — code 0. Versions, logs et sorties sont dans le dossier.

## Limites

Le test couvre le récepteur et le CLI réel du workspace dans Bubblewrap avec une fixture HTTP loopback privée, pas une instance Forgejo réelle. Il n’exerce pas une fusion ou un effet métier, ne teste pas une course d’écritures sur une même issue/PR, n’utilise pas LLM/AgentOS et ne compare pas les résultats à une CI conventionnelle. Le code de production ne change pas dans cette tranche.

Cargo reste `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`; aucune licence produit n’est choisie et les licences/droits des dépendances ne sont pas établis par ce garde.
