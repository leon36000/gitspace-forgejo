# CONTINUATION-043 — compatibilité et refus des alias de webhook

## Question vérifiée

Le receiver doit prendre en charge les en-têtes historiques `X-Gitea-*` seuls et refuser deux alias d’événement incompatibles, même si le HMAC du corps est valide. Les refus doivent laisser l’inbox durable inchangée.

## Réalisation et résultat

Deux tests démarrent le vrai binaire Rust `forgejo-webhook-local` avec un secret synthétique et un corps d’issue correctement signé.

- `actual_receiver_rejects_conflicting_forgejo_and_gitea_event_aliases_before_append` envoie `X-Forgejo-Event: issues` et `X-Gitea-Event: pull_request`. Le processus répond HTTP 400, `event_header_invalid`, `retryable=false`; après arrêt propre, l’inbox est vide.
- `actual_receiver_accepts_gitea_compatibility_headers_without_forgejo_aliases` convertit les trois champs de compatibilité (delivery, event et signature) vers `X-Gitea-*`, puis vérifie explicitement qu’aucun `X-Forgejo-*` n’est présent. Le processus répond HTTP 202, crée une unique entrée `forgejo.webhook.v1.issues` liée au delivery ID attendu.

La paire de tests distingue le comportement fail-closed et le chemin positif de compatibilité au niveau processus. Les handlers implémentaient déjà ces règles; aucune correction du code de production et aucune phase rouge n’étaient nécessaires pour ce lot de couverture.

## Commandes et environnement

Commande ciblée :

```text
cargo test --locked --offline --test webhook_receiver_process_e2e actual_receiver_rejects_conflicting_forgejo_and_gitea_event_aliases_before_append -- --exact --test-threads=1
cargo test --locked --offline --test webhook_receiver_process_e2e actual_receiver_accepts_gitea_compatibility_headers_without_forgejo_aliases -- --exact --test-threads=1
```

Résultat des deux commandes : chacune donne 1 réussi, 0 échoué, 87 filtrés; chacune s’exécute en 0,05 s. `cargo fmt --all -- --check` et `git diff --check` sortent avec le code 0. Les sorties sont archivées dans `conflicting-alias-test.log`, `gitea-only-compatibility-test.log`, `rustfmt-check.log` et `diff-check.log`. Versions et plateforme sont dans `toolchain.txt`; SHA-256 des sources pertinentes dans `source-sha256.txt`. `go-availability.log` consigne `go: command not found` (code 127).

## Portée et limites

Cette vérification couvre le processus local et une connexion loopback éphémère. Le serveur n’est pas Forgejo et ce test ne démontre pas un comportement en production, l’isolation réseau du processus, l’exactitude générale de toutes les règles d’événement, ni le parcours agent/LLM. Aucun test Go n’est possible dans l’environnement actuel (`go` absent du PATH). La comparaison contrôlée agent+CI conventionnels reste non faite.

Pas de changement de production, d’installation, de publication, de service externe, de secret réel ou de dépense. Le manifeste Cargo continue à ne choisir aucune licence et à indiquer les droits de redistribution comme non évalués.
