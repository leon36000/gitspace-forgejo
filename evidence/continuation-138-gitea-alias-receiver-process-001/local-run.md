# CONTINUATION-138 — alias Gitea seul au receiver processus

## Lacune et scénario

Le contrat HTTP accepte les familles d’en-têtes Forgejo et Gitea, mais le vrai receiver n’avait pas de test processus utilisant exclusivement les alias Gitea. Le test `tests/webhook_receiver_process_e2e.rs::actual_receiver_accepts_gitea_legacy_header_aliases` part d’une requête `issues` valide signée par le helper, puis remplace `X-Forgejo-Delivery`, `X-Forgejo-Event` et `X-Forgejo-Signature` par leurs équivalents `X-Gitea-*`. Il vérifie qu’aucun champ `X-Forgejo-*` ne reste.

Le vrai processus receiver accepte la requête HTTP 202 (`accepted=true`, `duplicate=false`, séquence 1). Le rejeu de l’inbox donne un record unique `forgejo.webhook.v1.issues` lié à `gitea-legacy-alias-41`. Les alias d’entrée sont normalisés vers le type journalisé canonique.

## Vérifications

Commande ciblée :

```text
cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_accepts_gitea_legacy_header_aliases -- --exact --nocapture
```

Résultat : 1 réussi, 0 échoué, 0 ignoré, 115 filtrés. `cargo fmt --all -- --check` et `git diff --check` passent. Seul un test processus est ajouté; le runtime et l’interface HTTP restent inchangés. Les hash locaux/worktree/projet ont été appendés; le vérificateur confirme les préfixes et le binding parent.

## Portée et limites

Le payload et la clé HMAC sont synthétiques, le receiver est local, et le test s’arrête après l’écriture durable. Il ne démarre ni Forgejo ni le CLI de réconciliation; il ne teste pas un hook Gitea live, les alias mixtes/dupliqués ou les proxys qui normalisent les en-têtes. Le parcours receiver→CLI avec les en-têtes canoniques Forgejo est enregistré séparément en CONTINUATION-137. Aucun LLM/AgentOS, secret privé, service externe, production, publication ou coût n’est impliqué. La mission reste incomplète.

## Révision et sources

HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Les commandes, versions, empreintes de source et snapshot de préfixe sont joints dans ce dossier.
