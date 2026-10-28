# CONTINUATION-389 — action d’erreur explicite pour `payload.is_pull`

Date : 28 octobre 2026. Branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Défaut et correction

Le chemin d’ingress rejetait bien un `issue_comment` sans `is_pull`, indiquait `details.field_path=payload.is_pull` et laissait l’inbox intacte. Cependant, son champ `error.action` demandait seulement d’inspecter le contrat, sans donner au consommateur les contraintes spécifiques nécessaires pour réparer l’événement.

Le handler fournit maintenant une action dédiée : `payload.is_pull` doit être un booléen JSON; `true` requiert `payload.pull_request.number == payload.issue.number`; `false` requiert l’absence de `payload.pull_request`; le consommateur peut consulter `GET /forgejo/capabilities` et renvoyer l’événement corrigé avec le même delivery ID. L’action rappelle que le corps rejeté n’a pas été journalisé. Le contrôle d’ingress n’a pas été assoupli.

## Vérifications

La commande `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_receiver_refuses_issue_comment_without_is_pull_before_reserving_delivery` a été exécutée d’abord avec l’assertion du nouveau contrat. Le run rouge échoue parce que `error.action` est générique; la sortie complète est `receiver-actionable-error-red.log`. Après le changement de `src/webhook/http.rs`, le test du vrai receiver-process sous Bubblewrap `--unshare-net --unshare-pid` passe 1/1 avec les assertions exactes sur le chemin, l’action, le refus non réessayable et le journal vide.

`cargo test --offline --locked --test webhook_handler_contract issue_comment_refuses_inconsistent_issue_pull_request_bindings -- --exact --test-threads=1` passe 1/1. `cargo fmt --all -- --check` et `git diff --check` passent. Les sorties vertes sont dans `handler-contract-green.log` et `receiver-actionable-error-green.log`; les commandes exactes sont dans `verification-commands.txt`.

## Limites

Seul le champ absent `is_pull` est exercé par ce processus. Les règles de `true`/`false` dans le conseil sont reliées aux tests de binding de CONT-386; elles ne sont pas chacune répétées ici. Le test utilise le receiver local et un payload de fixture, pas une instance Forgejo live. Aucun LLM, PR, merge ou benchmark n’est démontré; aucun service externe n’est utilisé.
