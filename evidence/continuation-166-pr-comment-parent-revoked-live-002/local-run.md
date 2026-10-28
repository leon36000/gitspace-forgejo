# CONTINUATION-166 — visibilité du parent PR révoquée avant readback

Date : 27 octobre 2026, 02:28 UTC. Exécution dans le worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` et modifications locales préexistantes préservées. Le runner a construit les binaires Rust sous Bubblewrap avec namespaces PID/réseau séparés et Cargo `--offline --locked`, puis a démarré une instance Forgejo jetable sur loopback.

## Question vérifiée

Forgejo peut masquer une ressource privée comme 404 après révocation d’un collaborateur. Pour un commentaire de PR supprimé, le reconciler doit-il annoncer `absent` lorsque le lecteur n’arrive plus à lire la PR parente? Le parcours vérifie ce cas pour l’ancien événement `created` et le nouvel événement `deleted`, avec une lecture indépendante par le propriétaire pour séparer la PR réellement supprimée d’une visibilité perdue.

## Commande

```sh
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root . \
  --state-dir /tmp/forgejo-pr-comment-parent-revoke-live-166-002 \
  --evidence-dir evidence/continuation-166-pr-comment-parent-revoked-live-002 \
  --delete-pull-request-comment \
  --revoke-pull-request-comment-reader-before-reconcile
```

## Résultat observé

Forgejo `17.0.0+gitea-1.22.0` a créé la PR #2, émis les livraisons Forgejo `issue_comment/created` et `issue_comment/deleted` pour le commentaire #4, puis émis la fermeture normale de PR. La suppression directe propriétaire avait répondu 204 et son GET commentaire répondait 404. Le runner a d’abord réconcilié les autres événements PR, puis le propriétaire a supprimé le collaborateur lecteur (DELETE 204).

Après révocation, le token reste authentifié (`GET /user` 200, login `forge-reader`, non-admin). Le GET du parent PR #2 avec ce même token répond 404; le propriétaire lit encore la PR #2 (HTTP 200, même URL). Le reconciler reçoit ensuite séparément chacun des deux delivery IDs : le `created` ancien et le `deleted` natif retournent tous deux le code de processus 1, `verdict=inconclusive`, `error.code=resource_not_found`, `retryable=false`, `upstream_read_attempts=1`, une identité liée au commentaire #4 / PR #2 et aucun `canonical_state`. Aucun résultat ne déclare le commentaire absent lorsque le parent est illisible.

Le reçu `result.json` inclut l’observation de révocation et les deux résultats. `pull-request-conversation-comment-stale-created-reconciliation.json` et `pull-request-conversation-comment-reconciliation.json` sont les reçus distincts; `webhook-events.json` et `forgejo-hook-tasks.json` lient événements et réponses 202. `port-cleanup-readback.json` confirme que le hook a été désactivé, les processus suivis arrêtés et les listeners retirés. Le premier essai avec un défaut d’import du harnais est conservé séparément : [CONTINUATION-166-RUN-001](../continuation-166-pr-comment-parent-revoked-live-001/local-run.md).

## Sources et vérifications

- Binaire Forgejo local : version `17.0.0+gitea-1.22.0`, Go `1.26.9`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le commit source exact incorporé dans ce binaire n’est pas établi. L’audit du dépôt source officiel reste le tag `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; aucun code Forgejo n’a été copié.
- HEAD était `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, mais le worktree était dirty; `provenance.json` conserve l’état Git et les hashes par fichier. Le hash du runner exécuté est `1830f7a8be9c87f7597e71137f41b40d663d78f2399eb66356a89132b754470d` et celui du fichier Python de tests est `eebc6f2cd43954b89f2cea5c18c7e4776eb7e2335379526defcc673b1b4bae28`.
- `PullRequestConversationCommentReconciliationTests` passe 8/8; le fichier entier `tests.test_forgejo_emitted_webhook_e2e` passe 120/120. `py_compile` des deux fichiers Python, `--help` du runner et `git diff --check` passent. Aucun test Rust n’a été relancé dans ce lot, qui ne modifie pas les sources Rust.
- Les fichiers binaires de l’adaptateur sont issus de Cargo 1.95.0 / rustc 1.95.0, sous Bubblewrap 0.9.0, avec réseau et PID isolés et source staged en lecture seule. Le reçu Cargo est dans `cargo-build.log`; hashes des exécutables dans `provenance.json`.
- Le manifeste local vérifie 23/23 fichiers. L’ajout append-only a ajouté 24 entrées à l’index du worktree et 25 à l’index du projet; les préfixes précédents sont préservés et l’index parent lie le manifeste worktree courant. Détails dans `manifest-check.txt` et `global-manifest-check.txt`.

## Limites

Run unique (`n=1`) sur une instance locale jetable, même UID hôte; le droit révoqué est celui d’un seul collaborateur lecteur; les appels API et vérifications sont séquentiels. Cela établit le résultat de visibilité/404 pour cette version/configuration, pas toutes les routes, versions ou politiques Forgejo, ni une garantie atomique contre une course de permissions. Ce n’est pas un test de production, de multi-tenant hostile, d’injection ou de client externe. Aucun LLM, AgentOS, RAG ou service de mémoire n’a été utilisé; aucun token/coût de modèle n’a été mesuré; aucun service ou donnée privé externe n’a été appelé. Le projet reste incomplet.
