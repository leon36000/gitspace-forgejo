# CONTINUATION-152 — cache du readback `issue_comment` au travers du CLI processus

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; aucune modification préexistante n’a été supprimée, commitée ou publiée.

## Scénario et résultat observé

Ajout d’un seul test processus ignoré par défaut, exécuté avec le runner ciblé existant `scripts/run_webhook_reconcile_cli_e2e.py`. Le scénario fait accepter au vrai receiver trois livraisons HMAC : une issue, puis `issue_comment.created` et `issue_comment.deleted` pour le même commentaire. Il lance le vrai binaire `forgejo-reconcile-local` contre une fixture HTTP locale qui renvoie `200` à l’issue parente et `404` au commentaire.

Le runner compile la cible et exécute le test sous Bubblewrap avec `--unshare-net --unshare-pid`. La seule API disponible est la fixture loopback. Résultat direct : **1 réussi, 0 échec, 120 filtrés**. Le CLI observe exactement deux requêtes, vers `/issues/24` et `/issues/comments/916`, avec zéro en-tête Authorization. Sa sortie structurée contient l’issue `current_state_read`, la livraison `created` en `inconclusive/resource_not_found`, puis la livraison `deleted` avec `comment.state=absent`; le lot annonce deux tentatives de lecture. Le corps du 404 et le texte de commentaire non fiable ne sortent pas. Le journal est identique octet pour octet avant et après la réconciliation.

Le test expose la composition que le contrat unitaire seul ne couvrait pas : receiver → journal persistant → CLI réel → fixture d’API. La correction runtime de normalisation de cache et son échec rouge antérieur sont documentés dans [CONTINUATION-151](../continuation-151-cross-action-comment-cache-001/local-run.md); aucun runtime n’a été modifié dans cette tranche.

## Vérifications exécutées

- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_shares_current_comment_read_across_create_delete_batch`: **exit 0**, 1/1.
- `rustfmt --edition 2024 tests/webhook_receiver_process_e2e.rs`: réussi.
- Parsing AST du runner Python : réussi.
- `git diff --check`: réussi avant mise à jour de ce reçu; la vérification finale est consignée dans `final-diff-check.log`.
- `python3 scripts/verify_cargo_license_state.py`: état vérifié après ce lot; aucune licence n’est sélectionnée, droits de redistribution non évalués.
- `append_manifests.py` puis `verify_append_only_indexes.py`: reçus vérifiés et préfixes des manifestes local, worktree et parent conservés.

Les commandes, versions, hashes des sources ciblées et reçus de vérification se trouvent dans les autres fichiers de ce dossier.

## Portée et limites

Il s’agit d’un scénario synthétique, avec une fixture loopback et un seul batch; ce n’est pas un serveur Forgejo réel, un parcours de projet/PR/merge, une exécution LLM ou AgentOS, une étude de performance ou une comparaison Forgejo+CI. Le test n’établit pas de gain global. Aucun secret réel, accès externe, coût, production, modification de Forgejo ou changement de licence n’est intervenu. La mission reste ouverte.

Suite : consulter la matrice des refus et choisir une lacune d’intégration encore sans preuve directe, sans relancer ce batch ni les parcours complets précédents.
