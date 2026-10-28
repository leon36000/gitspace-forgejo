# CONTINUATION-136 — doublon de l’en-tête d’événement HTTP

## Lacune examinée

`HttpRequest::header_alias` refuse les champs répétés pour un nom d’alias. La suite couvrait le doublon du champ de signature au niveau processus, mais pas un doublon de `X-Forgejo-Event` au vrai receiver. Le test ajouté porte sur cette frontière de parsing; aucune modification du runtime n’est justifiée par ce constat.

## Scénario et résultat

Le test `tests/webhook_receiver_process_e2e.rs::actual_receiver_rejects_duplicate_event_headers_before_append` lance le vrai receiver local. Il envoie une issue valide dont le corps est signé HMAC avec deux champs identiques `X-Forgejo-Event: issues`. Le receiver répond HTTP 400 `event_header_invalid`, `retryable=false`, et laisse le fichier d’inbox byte-for-byte identique. Il renvoie ensuite le même ID, corps et signature avec un seul champ : HTTP 202, `accepted=true`, `duplicate=false`, séquence 1. Le journal contient exactement un événement `forgejo.webhook.v1.issues` lié à cet ID.

## Vérifications

Les deux exécutions du test ciblé passent : 1 réussi, 0 échec, 0 ignoré, 113 filtrés. Le premier `cargo fmt --all -- --check` avait révélé un `assert_eq!` dépassant le format; cette sortie est conservée dans `rustfmt-initial.log`. Après une correction de mise en forme locale, le test ciblé, `cargo fmt --all -- --check` et `git diff --check` réussissent.

Commandes et sorties initiale/finale sont conservées dans le dossier. Les index locaux, worktree et projet sont append-only; `verify_append_only_indexes.py` confirme leur préfixe et leur liaison après append.

## Portée et limites

Corps et HMAC synthétiques, un receiver processus local, un seul cas de doublon identique. Ce test ne couvre pas une combinaison Forgejo/Gitea, un proxy qui normalise ou fusionne les en-têtes, tous les serveurs HTTP, une livraison Forgejo live, le CLI de réconciliation ou la concurrence. Aucun LLM/AgentOS, service externe, secret privé, production, publication ou coût n’est impliqué. Il s’agit d’un test de frontière d’entrée, pas d’une mesure de valeur ou d’un parcours agent complet. La mission reste incomplète.

## Révision et sources

HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Versions, empreintes de source et snapshots de préfixe sont joints dans `tool-versions.txt`, `source-hashes.txt`, `revision-snapshot.txt` et `manifest-prefix-before.json`.
