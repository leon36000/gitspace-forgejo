# CONTINUATION-229 — compatibilité des actions et statuts workflow

Date : 2026-10-27. Worktree local `webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dépôt fortement modifié. Aucun commit ni état propre n’est revendiqué.

## Constat et changement

Le contrat v3 exposait les trois valeurs d’action permises par ressource, mais n’exprimait pas leur compatibilité avec l’événement précis. Le source officiel Forgejo v17.0.0 montre que `run_status_changed` et `job_status_changed` sont émis pour les statuts non terminaux, tandis que `run_completed` et `job_completed` suivent `Status.IsDone()`. Cette distinction est maintenant publiée par `SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS` (14 associations) et `WEBHOOK_EVENT_CONTRACTS_V4`. `new_run_attempt` et `new_job_attempt` restent permis pour les deux classes : les méthodes notifier correspondantes constituent une voie séparée et le code de `NewWorkflowRunAttempt` transforme notamment un ancien état terminal avant l’envoi.

Le validateur contrôle maintenant l’action par ressource et par événement, à l’ingestion et lors du replay. Un couple incohérent devient `InvalidEventPayloadField("action")`, donc échoue fermé. Les anciennes formes v1–v3 restent publiées. Les capacités receiver passent à schema 5 et les capacités CLI à `forgejo-reconcile-capabilities-v9`, schema 9. Le message HTTP pour un payload invalide renvoie vers `GET /forgejo/capabilities` et précise que le corps refusé n’a pas été journalisé.

La régression a été observée avant le correctif : le cas `workflow_run_blocked` avec `run_completed` retournait `Ok`. L’assertion est maintenant dans le test de contrat couvrant les 14 événements, les actions admises, les actions inconnues et les associations terminales/non terminales incompatibles.

## Vérifications exécutées

- `cargo test --locked --offline --lib workflow_action_and_status_contracts_are_published_and_drive_payload_validation` : 1/1.
- `cargo test --locked --offline --test webhook_handler_contract` : 28/28.
- `cargo test --locked --offline --test webhook_reconcile_contract` : 77/77.
- Découverte de capacités CLI `actual_cli_discloses_versioned_local_test_capabilities` : 1/1.
- Endpoint du vrai processus receiver `capabilities_endpoint_describes_the_versioned_webhook_contract_without_mutating_inbox` : 1/1.
- `cargo test --locked --offline --all-targets` : 675 réussis, 0 échec, 126 ignorés, 36 résumés de cible.
- `cargo fmt --all -- --check`, `git diff --check`, vérificateur Cargo de licence et ses 12 tests : passent.
- `cargo metadata --offline --locked --no-deps --format-version 1` : `license=null`, `license_file=null`, `publish=[]`.

Les sorties de suite, le readback Cargo, les versions et les empreintes sont joints. Un premier `cargo fmt --all -- --check` a échoué sur trois mises en forme du lot incomplet; `cargo fmt --all` les a corrigées, puis le contrôle final a passé.

## Couverture et limites

L’examen Forgejo est un audit statique d’une version et d’un commit précis; les six fichiers sources sont inchangés par rapport au commit et leurs SHA-256 sont dans `forgejo-source-sha256.txt`. Aucun serveur Forgejo, runner Actions ou émission de tous les cas n’a été exécuté pour ce lot. Les tests prouvent le contrat local et ses consommateurs, pas la compatibilité de toutes les versions amont. Une action workflow n’est jamais une preuve indépendante de correction logicielle ni une autorité de fusion.

Ce lot ne lance aucun LLM et ne démontre aucune interface AgentOS utilisable ou valeur comparative. Aucun code Forgejo ou GitSpace n’est copié dans le produit. La licence du produit reste non choisie et les droits de redistribution non évalués. La mission demeure incomplète.
