# CONTINUATION-155 — doublon d’en-tête événement avec casse variable

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt reste modifié; aucun commit, reset ou nettoyage.

## Scénario et résultat

Le test existant `actual_receiver_rejects_duplicate_event_headers_before_append` rejette déjà deux champs identiques `X-Forgejo-Event`. Il vérifie maintenant aussi la requête signée contenant `X-Forgejo-Event: issues` et `x-fOrGeJo-eVeNt: pull_request`. Le receiver retourne HTTP 400 `event_header_invalid`, non réessayable, sans modifier l’inbox. Une seule ligne `X-Forgejo-Event` corrigée avec le même ID est ensuite acceptée en HTTP 202 et séquence 1; aucun ID n’a été réservé par les variantes rejetées.

Commande ciblée : `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_rejects_duplicate_event_headers_before_append -- --exact`. Résultat : **1 réussi, 0 échec, 121 filtrés**, test 0,08 s après compilation locale. Le processus ne parle qu’au receiver de test sur loopback; aucune requête externe n’est faite.

La vérification rend observable que les comparaisons de nom d’en-tête restent insensibles à la casse lors du comptage des occurrences, y compris lorsqu’une deuxième valeur contradictoire tente de changer `issues` en `pull_request`. La ligne correspondante de `docs/refusal-evidence-matrix.md` a été mise à jour.

## Vérifications

- Test ciblé ci-dessus : **exit 0**.
- `rustfmt --edition 2024 --check tests/webhook_receiver_process_e2e.rs`, `git diff --check`, garde Cargo et index append-only : réussis; reçus ci-joints.

## Limites

Corps et HMAC synthétiques, receiver local et un seul motif de doublon à casse variable. Le test ne simule pas un proxy qui fusionne, supprime ou réordonne des champs, ni une émission Forgejo/Gitea réelle. Le code runtime n’a pas changé. Aucune licence produit n’est sélectionnée, aucun LLM/AgentOS ou service externe n’a été utilisé, aucun coût ni changement production. Le projet reste incomplet.
