# CONTINUATION-439 — refuser l’absence quand le dépôt est illisible

Date : 28 octobre 2026. Worktree sur PC6, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Scénario et résultat

Le test processus `actual_reconcile_cli_process_refuses_branch_absence_when_repository_visibility_is_denied_or_hidden` est maintenant inclus dans l’allowlist du runner isolé. Après réception et journalisation d’un webhook natif `delete`, une première lecture de branche retourne 404. La seconde lecture, sur le dépôt configuré, est paramétrée successivement pour retourner 401, 403 et 404.

Dans chacun des trois cas, le CLI s’arrête après deux GET, ne relit pas la branche, renvoie `inconclusive` avec respectivement `upstream_unauthorized`, `upstream_forbidden` ou `resource_not_found`, et n’émet aucun `canonical_state`. Le test vérifie aussi l’absence d’en-tête Authorization, de marqueur privé dans stdout/stderr et de modification des octets de l’inbox.

## Vérifications

- Le scénario ignoré explicitement exécuté par `scripts/run_webhook_reconcile_cli_e2e.py` sous Bubblewrap 0.9.0 avec namespaces réseau et PID privés passe **1/1**; une exécution couvre les trois statuts.
- `cargo test --locked --offline --test webhook_receiver_process_e2e -- --test-threads=1` passe **44/44**, avec **164 ignorés** qui nécessitent leurs runners dédiés.
- `cargo fmt --all -- --check`, `git diff --check` et l’analyse syntaxique Python passent.
- L’audit des liens des documents canoniques passe **1 626/1 626**, sans cible manquante.
- Environnement, commandes, sorties, empreintes et codes de retour sont conservés dans ce dossier.

Le changement est limité à un scénario de refus et à son allowlist. Aucun code runtime ou comportement produit ne change.

## Limites

Les réponses HTTP sont produites par la fixture loopback. Cela n’établit pas la sémantique d’une instance Forgejo live, et le statut 404 du dépôt reste indistinguable d’un dépôt masqué par le serveur. Aucun agent/LLM, fusion ou mesure comparative n’est impliqué.
