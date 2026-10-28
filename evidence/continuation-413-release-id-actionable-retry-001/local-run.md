# CONTINUATION-413 — refus actionnable d’un `release.id` nul

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`; HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Défaut et correction

Le contrat v9 et `parse_resource_ref` validaient déjà que `release.id` est un entier JSON strictement positif. La route HTTP renvoyait bien 400 et n’ajoutait pas l’événement au journal quand l’ID était nul, mais l’action restait générique. Le premier test processus rouge échoue exactement sur le conseil : il obtient « inspect the event payload field… » au lieu d’une correction exploitable.

`src/webhook/http.rs` donne maintenant le conseil correspondant à `release.id` : fournir un entier JSON positif, consulter `GET /forgejo/capabilities`, puis redélivrer le même delivery ID; le corps refusé n’est pas journalisé. La règle d’admission n’a pas changé.

## Parcours et résultat

`actual_receiver_rejects_zero_release_id_then_accepts_same_id_retry` utilise le vrai receiver-process et une signature HMAC synthétique. La livraison signée avec `release.id=0` reçoit HTTP 400, `event_payload_invalid`, `retryable=false`, `details.field_path=payload.release.id`; le marqueur de corps n’apparaît pas dans la réponse et le replay du journal est vide. Le test corrige ensuite l’ID à 616 en gardant le même delivery ID. Le receiver répond HTTP 202, `duplicate=false`, sequence 1; le journal contient une seule livraison avec l’ID corrigé.

Résultat final : **1 passed, 0 failed, 0 ignored, 199 filtered out**, code de sortie 0, sous Bubblewrap `--unshare-net --unshare-pid`. Le test rouge, le run intermédiaire qui a détecté une assertion de test erronée, et le run final sont joints. `JournalEvent.event_id` est la séquence interne (`evt-1`); l’assertion a été corrigée pour vérifier le `payload.delivery_id` durable.

`cargo fmt --all -- --check`, `git diff --check`, l’AST du runner, la garde de licence Cargo et `cargo metadata --offline --locked --no-deps --format-version 1` passent. Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION`.

## Portée et limites

Changements : conseil d’erreur HTTP, test processus et entrée dans le runner. Pas de modification des règles de validation, dépendances, Cargo ou licence. Le receiver/HMAC est local et synthétique; aucun webhook natif ou Forgejo live n’est exercé. La nouvelle preuve couvre l’ID zéro, pas toutes les variantes de type et de débordement, ni un événement Release visant un autre dépôt. La suite complète n’a pas été relancée.
