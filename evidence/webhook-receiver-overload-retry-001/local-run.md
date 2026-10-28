# CONTINUATION-193 — 503 de surcharge et redélivraison exacte

Date : 27 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; dépôt fortement modifié.

## Lacune et correction

Le receiver a quatre workers, une file de seize connexions et un rejector séparé à une place. Sa réponse de surcharge `503/server_busy` était réessayable mais ne renvoyait pas `Retry-After`; les capacités ne décrivaient pas non plus cette erreur. Le test processus a d’abord échoué sur l’absence de cet en-tête après avoir rempli le pool et la file.

Le receiver fournit maintenant `Retry-After: 5` avec `server_busy`, un délai cohérent avec la fenêtre d’expiration des lectures incomplètes. L’endpoint `/forgejo/capabilities` décrit ce refus et son action; son `schema_version` et celui d’`error_contract` passent à 2. Le code n’interprète pas la livraison refusée et ne l’ajoute pas au journal.

## Parcours processus

`actual_receiver_refuses_when_connection_queue_is_full_then_accepts_exact_retry_once` démarre le vrai binaire receiver. Quatre connexions avec un corps partiel occupent les workers; seize probes GET complètes remplissent la file. La livraison HMAC valide suivante est refusée par le rejector : HTTP 503, `server_busy`, `retryable=true`, `Retry-After: 5`; l’inbox reste byte-for-byte inchangée.

Après le délai des quatre requêtes partielles, elles obtiennent 408 `request_timeout`, les seize probes obtiennent 405 `method_not_allowed`, puis le client redélivre exactement les mêmes octets signés. Le receiver répond 202 `sequence=1`, puis 202 `duplicate=true` au second retry. Après arrêt gracieux, le journal contient une seule entrée, celle de la livraison acceptée.

Le test rouge initial échoue sur le header manquant (`focused.log`). Après correction, le test processus passe trois fois : environ 3,11 s, 3,15 s et 3,09 s. Le test de capacités versionnées passe aussi.

## Vérifications

- `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_refuses_when_connection_queue_is_full_then_accepts_exact_retry_once -- --nocapture` — 1/1, puis deux répétitions 1/1.
- `cargo test --offline --locked --test webhook_receiver_process_e2e capabilities_endpoint_describes_the_versioned_webhook_contract_without_mutating_inbox -- --nocapture` — 1/1.
- `cargo test --offline --locked --all-targets` — 661 réussis, 0 échec, 124 ignorés sur 36 cibles.
- `cargo fmt --all -- --check`, `git diff --check` — passent.
- `cargo metadata --offline --locked --no-deps --format-version 1` et `scripts/verify_cargo_license_state.py` — `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, SPDX déclaré `NOASSERTION`.

Les sorties processus sont `focused.log`, `focused-green.log`, `focused-repeat-2.log`, `focused-repeat-3.log` et `capabilities-green.log`; la suite complète est `all-targets.log`. Versions, hashes et manifestes sont dans ce dossier.

## Portée et limites

Le test exerce le processus receiver, ses vraies capacités de workers/file/rejector et son journal append-only, avec des clients et signatures synthétiques sur loopback. Les tailles quatre/seize reflètent les constantes actuelles et devront suivre toute modification de capacité. Trois répétitions ne prouvent pas un SLO ou une charge prolongée. Aucun serveur Forgejo, readback par l’adaptateur, modèle/AgentOS, secret réel, service externe ou production n’est utilisé. Les capacités informent le client du délai; elles ne contraignent pas Forgejo à respecter ce délai. Aucune licence produit n’est choisie.
