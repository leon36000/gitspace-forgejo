# CONTINUATION-225 — statuts Actions publiés et consommés depuis le même contrat

Date : 27 octobre 2026. Worktree local fortement modifié; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Les fichiers de code et tests dans ce répertoire sont hachés après le changement. Aucun commit, publication, accès élargi, déploiement ou service externe.

## Changement

Les événements `workflow_run_*` et `workflow_job_*` étaient admis et les exigences de statut existaient dans le validateur, mais la relation entre en-tête, ressource, champ de payload et statut attendu n’était pas publiée comme données structurées. `src/webhook.rs::SUPPORTED_WORKFLOW_STATUS_CONTRACTS` décrit les 14 associations. `src/webhook/reconcile.rs::validate_event_payload_kind` consulte cette table pour refuser un statut différent; `valid_workflow_run_status` dérive aussi sa liste de la même source.

Le contrat historique `WebhookEventContractsV1` est conservé. L’enveloppe additive `WebhookEventContractsV2` publie le contrat d’événements courant et les 14 lignes Actions. Elle est incluse dans les capacités du receiver HTTP (`schema_version=3`) et celles du CLI (`schema_version=7`, `forgejo-reconcile-capabilities-v7`). Les tests vérifient pour chaque ligne que l’événement existe, que la ressource correspond, que son statut annoncé est accepté et qu’un autre statut est refusé; les interfaces publient les mêmes données.

## Vérification fraîche

Les commandes exactes et sorties figurent dans `commands.txt` et les fichiers `.log` associés. Chaque commande a terminé avec code 0.

- `cargo test --locked --offline --lib resource_kind_contract_tests` : 3/3.
- `cargo test --locked --offline --test webhook_reconcile_contract -- --test-threads=1` : 77/77.
- `cargo test --locked --offline --test webhook_handler_contract -- --test-threads=1` : 27/27.
- `cargo test --locked --offline --bin forgejo-reconcile-local` : 157/157.
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e -- --test-threads=1` : 5 réussis, 2 ignorés car fixés au port loopback 3130.
- Test exact `capabilities_endpoint_describes_the_versioned_webhook_contract_without_mutating_inbox` : 1/1 avec le vrai processus receiver local; 137 autres tests filtrés.
- `cargo fmt --all -- --check`, `git diff --check`, garde de licence Cargo et ses 12 tests : succès.

L’état Cargo relu par le garde est `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed` et `NOASSERTION`. Aucun choix de licence n’est introduit.

## Limites

Les preuves sont des tests unitaires/contrats, tests du CLI local et un GET de capacités sur le processus receiver. Elles ne lancent ni Forgejo ni ses runners et ne vérifient pas les payloads réellement émis par chaque version Forgejo. Un statut Actions demeure une observation issue du webhook; il ne certifie ni le commit, ni la qualité du logiciel, ni une décision de merge. Aucun LLM, AgentOS, modèle externe ou benchmark comparatif n’est utilisé. Les tests n’ajoutent aucune preuve d’amélioration générale ou de comportement utilisateur. Le projet reste incomplet.
