# CONTINUATION-151 — cache batch du readback des commentaires

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree reste modifié; aucun commit, reset, publication ou nettoyage n’a été effectué. Versions et empreintes : `toolchain-and-head.txt` et `source-sha256.txt`.

## Défaut observé et correction

Un commentaire `created` et le même commentaire `deleted` ont le même endpoint Forgejo, mais `ResourceId::IssueComment` inclut le booléen `deleted_event`. `CachedCurrentState` s’en servait directement comme clé; le batch lisait donc le même endpoint deux fois et pouvait obtenir deux snapshots différents pour un même objet.

Le test `batch_reconciliation_reuses_comment_read_across_create_delete_and_parent_issue` a d’abord échoué avec trois lectures observées (issue, commentaire `deleted_event=false`, commentaire `deleted_event=true`). Le cache normalise maintenant cette valeur uniquement pour l’identité de lecture et appelle le reader avec cette identité. La valeur de l’action originale reste dans le `ResourceRef` reçu par le reconciler; elle décide toujours comment interpréter un `404`.

Le test envoie un webhook d’issue, puis `created`/`deleted` pour le commentaire 29, et répète l’ordre inverse. Chaque ordre effectue exactement un GET de l’issue parente et un GET du commentaire. La création reste `Source(NotFound)`; la suppression rend `comment.state=absent`. Aucun résultat n’est promu depuis le payload du webhook.

## Vérifications

- `cargo test --offline --locked --test webhook_reconcile_contract`: **57 réussis, 0 échec** (`cargo-reconcile-contract.log`).
- `cargo test --offline --locked --all-targets --quiet`: **631 réussis, 0 échec, 111 ignorés sur 36 blocs de résultat (`cargo-all-targets.log`, agrégat JSON à côté).
- `cargo fmt --all -- --check` et `git diff --check`: réussis.
- Garde Cargo réussie : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`; aucun choix de licence produit.

## Portée et limites

Le test est contractuel avec une fixture `ForgejoCurrentState`; le batch webhook signé→inbox→adapter HTTP de Forgejo n’a pas été rejoué. Le changement limite les lectures API redondantes et rend le snapshot commun aux actions pour le commentaire concerné; il ne prouve pas l’ordre global des événements, la durabilité, une isolation générale ni un gain mesuré. Aucun LLM/AgentOS, instance Forgejo live, service externe, secret réel ou coût n’a été utilisé. Le projet reste incomplet.

Suite : conserver la clé de cache fondée sur l’identité de l’endpoint, puis intégrer ce readback au prochain parcours composé uniquement après audit des tests de batch existants; ne pas répéter les runs MISSION-012 déjà prouvés. Le benchmark contrôlé et l’exécution LLM autorisée restent ouverts.
