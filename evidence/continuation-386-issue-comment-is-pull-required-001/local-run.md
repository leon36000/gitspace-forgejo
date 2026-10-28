# CONTINUATION-386 — `issue_comment.is_pull` obligatoire

Date : 28 octobre 2026. Worktree de produit : `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Ce reçu décrit le lot présent, pas l’ensemble des changements non commités du worktree.

## Source et lacune

Forgejo v17.0.0 a été inspecté localement dans le scratch officiel au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. `modules/structs/hook.go::IssueCommentPayload.IsPull` (lignes 204–213, blob `9c7f1deed1c38eeb435ab976bb8429c5458c7f2b`) est un `bool` marqué `json:"is_pull"`, sans `omitempty`. Le fichier source est propre dans le scratch. Cette forme source étaye le contrat; elle ne constitue pas un essai d’émission de Forgejo live.

Le test de processus sauvegardé `red-actual-receiver.log` a d’abord montré le défaut : l’exécutant reçoit HTTP 202 pour le payload signé sans `is_pull` et l’ID de livraison est journalisé. La validation locale a ensuite rendu le booléen obligatoire, exige `pull_request` lorsque sa valeur est vraie et l’interdit lorsqu’elle est fausse. Le schéma machine courant devient v8; les contrats v1–v7 utilisent l’ancienne table figée.

## Vérifications après correction

Toutes les commandes utilisent le cache Cargo local et `--offline --locked`; voir `verification-commands.txt`.

- `webhook_handler_contract::issue_comment_refuses_inconsistent_issue_pull_request_bindings` : 1/1.
- `webhook_reconcile_contract::issue_comment_events_require_supported_action_and_positive_comment_id` : 1/1; le champ absent ou non booléen est refusé avant tout readback API.
- `webhook_handler_contract::capabilities_publish_machine_readable_contracts_for_every_supported_event` : 1/1; v7 reste inchangé et v8 annonce `is_pull (boolean)`.
- `forgejo_reconcile_capabilities_e2e::actual_cli_discloses_versioned_local_test_capabilities` : 1/1.
- Receiver-process refusant l’absence de `is_pull` : 1/1 sous Bubblewrap `--unshare-net --unshare-pid`; HTTP 400 `event_payload_invalid`, `retryable=false`, journal vide, sans réservation de l’ID.
- Receiver-process acceptant un commentaire de PR valide v17 : 1/1, HTTP 202.
- `cargo fmt --all -- --check`, `git diff --check`, l’AST Python du runner et `cargo metadata --offline --locked --no-deps` : réussite.

Un premier contrôle de l’assertion de capacités a échoué parce que le test cherchait le texte exact `is_pull` tandis que le schéma documente le type comme `is_pull (boolean)`; l’assertion a été corrigée et le test vert est archivé. Le premier contrôle rustfmt a aussi demandé uniquement la mise en forme des deux assertions ajoutées; `cargo fmt --all` l’a appliquée et le contrôle final passe. Ces deux sorties diagnostiques sont conservées séparément.

## État du dépôt et limites

Les métadonnées Cargo restent `license=null`, `license_file=null`, `publish=[]`; aucune licence n’est choisie. Aucun code Forgejo ou GitSpace n’est copié. Les tests de receiver/HTTP et les lectures upstream sont locaux; ils ne démontrent pas un événement émis par une instance Forgejo vivante, AgentOS, Forge Code, un agent LLM, une PR ou fusion GitLab, ni un gain par rapport à une baseline. Le transfert PC4 reste séparément bloqué par le DNS/routage établi en [CONTINUATION-385](../continuation-385-gitlab-pc4-preflight-credential-scope-001/local-run.md); aucune authentification ou écriture distante n’a été effectuée dans ce lot.

Prochaine action pour ce contrat : ajouter les refus de types et de binding au parcours de retry même-ID lorsque le système reçoit un payload malformé après redémarrage, sans relâcher le rejet initial et sans répéter le scénario déjà établi. Pour GitLab, attendre une route HTTPS joignable depuis PC6 et un credential minimal pour le seul dépôt, puis lire les références distantes avant toute écriture.
