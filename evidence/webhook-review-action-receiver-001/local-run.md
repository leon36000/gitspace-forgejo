# CONTINUATION-123 — receiver processus refuse un avis PR sans `action=reviewed`

Date : 2026-10-26. Worktree de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, état fortement modifié. Aucun reset, commit, staging, service externe ou accès production.

## Question examinée

CONTINUATION-122 avait établi le rejet dans le handler et pendant la relecture du journal, mais ne couvrait pas le processus du binaire receiver. Cette preuve ciblée vérifie que le vrai `forgejo-webhook-local` applique la même règle d’admission et ne réserve pas un ID avant la validation de `action`.

## Exécution

Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_receiver_requires_reviewed_action_before_reserving_delivery`. Le runner compile le test d’intégration et les binaires configurés dans une copie source sous Bubblewrap, puis exécute exactement le scénario avec `--exact --ignored --nocapture`. Les namespaces réseau et PID sont isolés; le test receiver échange uniquement avec un port loopback éphémère. Cargo est en mode `--offline --locked`.

Le test démarre le binaire `forgejo-webhook-local` avec environnement vidé et un secret local synthétique. Il lui envoie un payload `pull_request_approved` signé correctement par le fixture, contenant `review.type=pull_request_review_approved` mais sans `action`. Le receiver répond HTTP 400, `event_payload_invalid`, `payload.action`, `retryable=false`; la relecture de l’inbox est vide. Il renvoie ensuite le même ID de livraison avec le seul champ `action="reviewed"` ajouté. Le receiver répond HTTP 202, `accepted=true`, `duplicate=false`, séquence 1; l’inbox contient une ligne de type `forgejo.webhook.v1.pull_request_approved`.

Résultat final : **1 test passé, 0 échoué, 111 filtrés**, durée rapportée 0,06 s. Le log initial montre également un premier passage vert avant rustfmt; le contrôle rustfmt de ce passage avait signalé une assertion trop longue. La mise en forme a été corrigée, puis le même scénario a été relancé : le log final est vert. `cargo fmt --all -- --check`, le contrôle Python AST du runner et `git diff --check` finaux passent.

## Portée et limites

Il s’agit d’un test processus du receiver Rust réel, pas d’un test de handler. Le corps et son HMAC sont synthétiques; seul `pull_request_approved` est couvert à ce niveau. CONTINUATION-122 vérifie séparément les trois sous-types d’avis et actions invalides au niveau handler, et le replay au niveau contrat. Aucune instance Forgejo n’a émis le webhook. Ce résultat ne porte pas sur l’authenticité de l’émetteur au-delà du secret fixture, les permissions de review, l’autorité de fusion, un runner Actions, un LLM/AgentOS ou une amélioration de productivité. `n=1`; aucun coût modèle ou token.

Versions, commandes, sorties et empreintes sont dans les fichiers du présent dossier. Le répertoire conserve le premier contrôle de format échoué comme historique et distingue explicitement le contrôle final réussi. Le manifeste local et les index worktree/projet sont append-only; le vérificateur final confirme leurs préfixes et leurs dernières empreintes. La mission globale reste incomplète.
