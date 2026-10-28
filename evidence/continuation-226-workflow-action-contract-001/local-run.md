# CONTINUATION-226 — actions webhook versionnées et consommées par le validateur

Date : 27 octobre 2026. Worktree local fortement modifié à HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le diff préexistant a été conservé; aucun commit, publication, déploiement, coût ou service externe.

## Changement

Dans les branches `workflow_run` et `workflow_job`, les trois noms de `action` admis étaient des chaînes privées au validateur. `src/webhook.rs::SUPPORTED_WORKFLOW_ACTION_CONTRACTS` publie maintenant pour chaque ressource `payload_field=action` et la liste admise. `src/webhook/reconcile.rs::validate_event_payload_kind` sélectionne ce contrat et rejette une action absente. Les 14 événements de statut conservent le comportement d’actions autorisées préexistant.

`WebhookEventContractsV3` ajoute la table aux métadonnées machine lisibles, tandis que les enveloppes v1 et v2 demeurent publiées. Le receiver a `schema_version=4`; le CLI annonce `schema_version=8` et `forgejo-reconcile-capabilities-v8`. Les tests vérifient la table exacte, le lien de chaque statut à sa ressource, toutes les actions déclarées et le refus d’une action future inconnue.

La liste admise est recoupée avec Forgejo officiel v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571` : `modules/structs/hook.go:521-523` et `:543-545` déclarent les mêmes six valeurs. Le fichier a le SHA-256 `42f951d98dbea6b78e70e595b8c2cc0f5931bee2d96b837d8d4e8f034b015a12` et n’a aucun diff vis-à-vis du commit dans le scratch local; le reste du scratch Forgejo demeure modifié pour des POC sans rapport avec ce contrat. Reçu : `forgejo-v17-action-source-review.md`.

## Vérification fraîche

Les commandes exactes et versions figurent dans `commands.txt` et `tool-versions.txt`; les sorties détaillées sont les `.log` joints. Codes de sortie : 0 pour chaque commande.

- `cargo test --locked --offline --lib resource_kind_contract_tests` : 3/3.
- `cargo test --locked --offline --test webhook_reconcile_contract -- --test-threads=1` : 77/77.
- `cargo test --locked --offline --test webhook_handler_contract -- --test-threads=1` : 27/27.
- `cargo test --locked --offline --bin forgejo-reconcile-local` : 157/157.
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e -- --test-threads=1` : 5 réussis, 2 ignorés car attachés au port loopback 3130.
- `capabilities_endpoint_describes_the_versioned_webhook_contract_without_mutating_inbox` : 1/1 avec le processus receiver local; 137 autres tests filtrés.
- `actual_reconcile_cli_process_reads_workflow_run_canonically_and_projects_safe_fields` et `actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details` : chacun 1/1 via `scripts/run_webhook_reconcile_cli_e2e.py`, Bubblewrap, namespaces réseau/PID isolés et fixture HTTP loopback. Le receiver/journal/CLI sont les binaires du projet; Forgejo et ses runners ne sont pas lancés.
- Formatage, `git diff --check`, garde Cargo et ses 12 tests : succès.

Cargo demeure sans licence choisie : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, `NOASSERTION`.

## Limites

Ces preuves vérifient notre contrat local et le processus du receiver; elles ne lancent pas Forgejo ni ses runners et n’attestent pas les valeurs envoyées par toutes les versions Forgejo. L’action reçue demeure consultative. Aucun LLM, AgentOS, modèle ou benchmark comparatif n’est utilisé. Les problèmes d’intégrité historiques des index globaux relevés en CONT-225 restent présents; le nouveau segment d’index sera vérifié séparément. Le projet reste incomplet.
