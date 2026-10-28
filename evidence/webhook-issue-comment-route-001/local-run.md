# WEBHOOK-ISSUE-COMMENT-ISSUE-ROUTE-001 — commentaire d’issue vers l’état courant

## Contexte et périmètre

Source testée dans le worktree local, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree est modifié et contient des fichiers non suivis; les empreintes `source-sha256.txt` fixent les sources utilisées pour ce scénario. Aucun code Forgejo n’a été copié ou modifié dans ce lot.

Le récepteur local et le CLI `forgejo-reconcile-local` sont exécutés comme binaires du workspace. Le serveur HTTP de l’API est une fixture loopback privée, réservée au scénario; le test complet est lancé sous Bubblewrap avec `--unshare-net`. Aucun service externe, secret réel, Forgejo de production, modèle ou AgentOS n’est utilisé.

## Comportement vérifié

Le test `actual_reconcile_cli_process_routes_issue_comment_to_issue_api` envoie au processus récepteur une livraison synthétique signée correspondant à la forme Forgejo v17 d’un commentaire sur une issue ordinaire : `issue.number=24`, `is_pull=false`, et aucun objet `pull_request`. Le récepteur accepte la livraison et le readback confirme une ligne journalisée.

Le processus CLI ouvre ensuite l’enveloppe journalisée et émet un seul `GET /api/v1/repos/forge-admin/agent-forge-core/issues/24`. Il n’appelle pas `/pulls/24` et n’envoie pas d’en-tête `Authorization`. La notification signée annonce `open`; l’API fixture annonce `closed`. La sortie CLI est `current_state_read`, ressource `issue` #24, état `closed`, avec `updated_at` et URL canonique issus de la réponse actuelle.

Ce cas complète le scénario processus existant pour un commentaire de conversation PR (`is_pull=true`, objet `pull_request`, route `/pulls/23`), sans rejouer ce dernier. L’inspection montre que `resource_kind_for_event` supportait déjà la branche issue; aucune correction de code de production n’a été nécessaire. La contribution de ce lot est la preuve intégrée jusque-là manquante et son runner ciblé.

## Commandes et résultats

- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_routes_issue_comment_to_issue_api` : 1/1, code 0.
- `cargo test --offline --locked --test webhook_handler_contract` : 12/12, code 0.
- `cargo test --offline --locked --test webhook_reconcile_contract` : 23/23, code 0.
- `python3 scripts/test_cargo_license_state.py` : 7/7, code 0.
- `python3 scripts/verify_cargo_license_state.py` : `verified`, code 0.
- `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, `cargo fmt --all -- --check` et `git diff --check` : code 0.

Les sorties et codes de retour sont conservés dans ce dossier. Les versions et l’état exact des sources sont dans `tool-versions.txt` et `source-sha256.txt`.

## État de la licence Cargo

Le garde relit `cargo metadata --locked --offline --format-version 1 --no-deps` et confirme `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected` et `redistribution_rights=not_assessed`. Cela rend l’absence de choix explicite et vérifiable, mais n’accorde aucun droit et n’évalue pas les licences des dépendances. Aucune licence n’est sélectionnée.

## Limites

Ce test vérifie le comportement des exécutables du workspace contre un serveur HTTP fixture, pas la compatibilité runtime avec une instance Forgejo réelle pour cet événement, ni l’effet utilisateur du commentaire. Le test de PR adjacent reste une preuve précédente et n’a pas été rejoué. Il n’y a ni LLM, ni AgentOS, ni décision de fusion, ni mesure de qualité ou de coût. Ce succès de test ne démontre pas un gain général de la forge.
