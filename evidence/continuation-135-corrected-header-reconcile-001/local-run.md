# CONTINUATION-135 — réconciliation du rejeu après correction d’en-tête

## Scénario

Le test `tests/webhook_receiver_process_e2e.rs::actual_receiver_rejects_a_signed_body_with_a_mismatched_event_header_before_append` a été prolongé après le scénario CONTINUATION-134. Le vrai processus receiver local refuse HTTP 400 le corps `issues` signé avec l’en-tête non signé `pull_request` (`event_payload_invalid`) et laisse l’inbox vide. Le même identifiant, corps et HMAC avec l’en-tête `issues` est ensuite accepté en HTTP 202, `duplicate=false`, séquence 1. Le rejeu donne exactement un record `forgejo.webhook.v1.issues` pour ce delivery ID.

Le test passe ensuite ces événements à `reconcile_inbox_events` et au `FixtureCurrentState` existant. Le résultat est lié à `Issue/41`, état `open` et URL fournie par le fixture; un seul readback est enregistré. Cela vérifie que le type corrigé atteint la ressource de réconciliation attendue dans ce scénario, après que le type erroné a été refusé sans réserver l’ID.

## Vérifications

Commande ciblée :

```text
cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_rejects_a_signed_body_with_a_mismatched_event_header_before_append -- --exact --nocapture
```

Résultat : 1 réussi, 0 échoué, 0 ignoré, 112 filtrés. `cargo fmt --all -- --check` réussit. `git diff --check` réussit après la mise à jour du checkpoint et de la matrice. Aucun code runtime n’a changé; seul le test processus existant a été prolongé.

## Portée et limites

Le receiver est un processus local réel, mais le corps et la clé HMAC sont synthétiques. La réconciliation est appelée directement dans le test via `FixtureCurrentState`: aucun CLI réconciliateur ni appel HTTP à Forgejo n’est exécuté par cette extension. Elle ne prouve donc ni l’émission native par Forgejo, ni l’intégration CLI/API, ni une défense concurrente générale. Aucun LLM, AgentOS, service externe, secret privé, production ou coût n’est impliqué. Le résultat est un test d’intégration local à un cas, pas un gain comparatif. La mission demeure incomplète.

## Révision et artefacts

HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; voir `revision-snapshot.txt`, `tool-versions.txt`, `commands.txt`, journaux et `source-hashes.txt`. Les manifestes locaux/worktree/projet sont append-only; leurs préfixes et bindings sont vérifiés par `verify_append_only_indexes.py`.
