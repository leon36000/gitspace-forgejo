# WEBHOOK-022 — pipeline de réception à réconciliation

Date : 2026-10-22. Worktree local `local/WEBHOOK-005-canonical-links`, HEAD de départ `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours exercé

Le contrat d’intégration `tests/webhook_reconcile_pipeline.rs` compose le parseur HTTP, le handler HMAC et son `EventJournal` append-only, la relecture vérifiée du journal et `reconcile_inbox_events`. Le payload d’issue synthétique annonce `closed`; le snapshot retourné par `FixtureForgejoCurrentState` annonce `open`. Le résultat canonique doit être `open`. Le test vérifie aussi la livraison dupliquée idempotente, le conflit non réessayable lorsqu’un même delivery ID revient avec un autre corps valablement signé, la conservation d’un nouvel ID, l’ordre des deux reçus et le partage d’une seule lecture de l’état courant.

Toutes les données Forgejo et la clé HMAC sont des fixtures de test. La fixture implémente directement le trait `ForgejoCurrentState`: aucun socket ni serveur Forgejo n’est appelé. Le test passe par `parse_http_request` et `handle_webhook_request`, mais ne démarre pas le binaire TCP `forgejo-webhook-local`. Ce résultat complète un contrat d’intégration de bibliothèque; il ne constitue ni un smoke Forgejo live, ni le parcours PR/fusion, ni une exécution de LLM.

## Vérifications reproductibles

Depuis la racine de ce worktree :

```sh
CARGO_TARGET_DIR=/tmp/webhook-reconcile-pipeline-target unshare -rn cargo test --offline --locked --test webhook_reconcile_pipeline -- --nocapture
CARGO_TARGET_DIR=/tmp/webhook-reconcile-pipeline-target cargo test --offline --locked
cargo fmt --all -- --check
git diff --check
```

Résultats : contrat ciblé 1/1; suite Cargo 199 réussis, 0 échec, 2 helpers ignorés; rustfmt et `git diff --check` réussissent. La première suite complète sous `unshare -rn` a échoué dans sept tests du client HTTP fixture parce que cette isolation rendait son loopback indisponible. La suite a été relancée hors de l’espace réseau, avec Cargo hors ligne; les tests n’ont contacté que leurs serveurs locaux éphémères. Les deux sorties complètes sont conservées afin de montrer le blocage et le rerun réussi.

Journaux : `webhook-reconcile-pipeline-003-focused.log`, `webhook-reconcile-pipeline-004-full-tests-loopback.log`, `webhook-reconcile-pipeline-003-rustfmt.log`, `webhook-reconcile-pipeline-003-diff-check.log` et le premier échec `webhook-reconcile-pipeline-003-full-tests.log`.

## Suite utile et périmètre Forgejo constaté après le test

L’absence de Forgejo sur le port attendu par l’adaptateur (`3130`) est confirmée, mais une autre instance locale existe sur `127.0.0.1:3132`. À 20:37 UTC, son `GET /api/healthz` répondait 200 et une lecture API authentifiée confirmait Forgejo `17.0.0+gitea-1.22.0`; l’instance déclare `offline_mode=true`, SSH, Actions et Packages désactivés. Les lectures API anonymes du dépôt de test privé répondaient 403. Aucun changement n’a été envoyé à l’instance. La preuve avec secrets omis est [webhook-reconcile-pipeline-005-forgejo-readback.json](webhook-reconcile-pipeline-005-forgejo-readback.json); l’inventaire d’instance initial est [forgejo-instance-scope.json](../../../evidence/mission-012-ollama-runtime/raw/forgejo-instance-scope.json).

Le code du client `forgejo-reconcile-local` reste volontairement sans authentification et épinglé à `http://127.0.0.1:3130` (`src/bin/forgejo-reconcile-local.rs`, constantes et commentaire `LocalLoopbackForgejoApi`). Il ne peut donc pas lire le dépôt privé sur 3132. L’étape suivante est de concevoir une source d’authentification strictement hôte, sans remettre le credential à l’exécutant ni exposer son contenu aux arguments/processus; jusqu’à cette frontière, garder la réconciliation sur fixture. La vérification indépendante PR/fusion est déjà documentée séparément sous MISSION-012 et reste semi-manuelle.
