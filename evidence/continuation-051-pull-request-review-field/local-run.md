# CONTINUATION-051 — rejet du champ `review` sur l’événement `pull_request`

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le worktree était déjà fortement modifié; aucune modification préexistante n’a été réinitialisée ou remplacée.

## Défaut et correction

Dans `src/webhook/reconcile.rs`, `validate_event_payload_kind` rejetait les champs `issue` et `comment` pour l’événement `pull_request`, mais seulement les sous-types de revue connus entraînaient ensuite un refus. Ainsi `review: null`, un scalaire, `{}`, un type mal formé ou une nouvelle valeur passait la validation. La réconciliation pouvait appeler Forgejo; le receiver signait correctement le corps et pouvait répondre 202 puis l’ajouter au journal. Les tests rouges consignés dans `red-phase.txt` reproduisent ces comportements.

Le validateur traite maintenant toute présence top-level de `review` comme un champ inattendu sur `pull_request`, avant l’extraction de la ressource ou toute lecture API. La découverte du contrat dans `src/webhook.rs` décrit le même refus. Les en-têtes dédiés `pull_request_approved`, `pull_request_rejected` et `pull_request_comment` continuent d’accepter leurs propres payloads `review.type`.

## Vérification

Les tests ajoutés dans `tests/webhook_reconcile_contract.rs` couvrent null, booléen, chaîne, tableau, objet vide, sous-type futur, type non chaîne et sous-type connu; ils vérifient une erreur `UnexpectedEventPayloadField("review")` et zéro appel API. Le test dans `tests/webhook_handler_contract.rs` utilise des corps HMAC valides; il vérifie HTTP 400, code `event_payload_invalid`, chemin `payload.review` et journal vide.

La vérification finale est dans [le log all-targets CONTINUATION-052](../continuation-052-merge-readback-base-identity/all-targets-final.log), qui couvre aussi cette tranche : 576 réussis, 0 échec, 85 ignorés, 34 cibles de test. Le contrôle de format et `git diff --check` sont enregistrés avec CONTINUATION-052.

## Portée

C’est une correction de contrat Rust, validée avec des fixtures locales. Aucun serveur Forgejo, LLM, AgentOS, credential ou service externe n’a servi pendant cette tranche. Elle ne démontre pas la sécurité d’un déploiement ou un gain comparatif.
