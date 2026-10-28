# CONTINUATION-381 — cohérence de liaison issue_comment vérifiée au receiver

Date : 28 octobre 2026. Worktree local HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0.

## Lacune observée

La suite handler avait un test nommé issue_comment_refuses_inconsistent_issue_pull_request_bindings, mais les payloads invalides omettaient tous action. tests/webhook_handler_contract.rs::signed_request sérialise le JSON fourni sans ajouter de champ. Dans src/webhook/reconcile.rs::validate_event_payload_kind, l’action est contrôlée avant les champs issue/pull_request/is_pull. Ainsi, l’ancien test pouvait ne jamais atteindre les assertions de liaison malgré le statut HTTP 400 attendu.

## Vérification ajoutée

Les cas négatifs existants ont reçu action=created, une action Forgejo valide. Le lot couvre maintenant un numéro de PR différent de l’issue, un pull_request null avec is_pull=false, un pull_request absent avec is_pull=true et un is_pull de type non booléen. Le test construit une requête signée HMAC avec la fixture locale, appelle le handler, exige HTTP 400 et vérifie que le journal reste vide. Le test direct de réconciliation ajoute l’assertion précise ResourceMismatch pour is_pull=true sans pull_request.

Le garde-fou de production existait déjà; aucun code produit n’a changé dans ce lot. L’ancienne fixture invalide a été repérée par inspection source; aucun run rouge antérieur n’a été capturé. Le progrès consiste à ce que les tests traversent désormais l’étape d’action avant d’éprouver les liaisons.

Source Forgejo officielle : v17.0.0 au commit 0d791421f85ca13a63b665053d81ea765fbbc571, modules/structs/hook.go::IssueCommentPayload, lignes 203–213. Le worktree scratch local est à ce commit et ce fichier source ne présente pas de modification locale : PullRequest est omis lorsqu’il est nul et IsPull est un booléen.

## Vérifications

- cargo test --offline --locked --test webhook_handler_contract : 30 réussis, 0 échec.
- cargo test --offline --locked --test webhook_reconcile_contract : 98 réussis, 0 échec.
- cargo fmt --all -- --check et git diff --check : réussis.
- scripts/test_cargo_license_state.py : 12/12.
- cargo metadata --offline --locked --no-deps confirme license=null, license_file=null, publish=[].

Les journaux complets et la réponse Cargo sont conservés ici. Le fichier LICENSE n’est pas choisi et aucun droit de redistribution n’est déduit.

## Limites

Le test receiver utilise handle_webhook_request en mémoire et une clé HMAC synthétique; le test reconciler utilise un JournalEvent fixture. Aucun binaire receiver isolé, Forgejo vivant, émission de webhook native, AgentOS/LLM, PR, fusion contrôlée ou mesure comparative n’est exercé. Le comportement après fusion n’est pas concerné. Cette tranche ferme un défaut de couverture; elle ne démontre pas seule la fiabilité du chemin complet.

Suite : examiner les autres tests d’ingestion pour les préconditions manquantes avant leurs assertions sémantiques, en privilégiant un chemin processus réellement absent des preuves.
