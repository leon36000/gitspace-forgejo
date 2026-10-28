# FORGEJO-LIVE-COMMENT-DELETE-READBACK-001 — absence canonique après suppression

Date : 26 octobre 2026. Résultat : **réussi sur une instance Forgejo locale jetable, n=1**.

## Lacune ciblée

Le rapport [FORGEJO-LIVE-ORDINARY-ISSUE-COMMENT-LIFECYCLE-002](../forgejo-live-ordinary-issue-comment-lifecycle-002/local-run.md) vérifiait le `204` du DELETE et la livraison du webhook `deleted`, mais n'effectuait pas de GET canonique après suppression. Ce lot ajoute une seule observation nouvelle : relire le même identifiant via l'API Forgejo après le DELETE.

## Source et provenance

- Forgejo source : tag/version `v17.0.0`, commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`, dans le scratch `/tmp/forgejo-v17-base-cas-poc`. Le chemin ciblé `routers/api/v1/repo/issue_comment.go` correspond au blob `92d7d714a478d41b68580ef2cace698b74c7155c`; le fichier ciblé est inchangé par rapport au commit. L'examen porte sur `GetIssueComment` et sa route `GET /repos/{owner}/{repo}/issues/comments/{id}`, pas sur tout Forgejo.
- Le binaire local annonce Forgejo `17.0.0+gitea-1.22.0`, Go `1.26.9`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Son commit de compilation n'est pas établi.
- Le worktree produit était au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, préexistant fortement modifié. Les hashes exacts des scripts/tests utilisés sont dans `provenance.json`; le reçu indique explicitement les chemins non suivis et l'état sale de départ.

## Exécution et résultat

Commande exécutée depuis la racine produit :

```sh
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-comment-delete-readback-001 \
  --evidence-dir evidence/forgejo-live-comment-delete-readback-001 \
  --ordinary-issue-comment \
  --edit-ordinary-issue-comment \
  --delete-ordinary-issue-comment
```

Forgejo a créé, édité puis supprimé le commentaire `#6` de l'issue `#1`; les mutations attendues ont retourné `201`, `200` et `204`. Après le `204`, avant l'attente du webhook de suppression, le runner appelle `GET /repos/forge-admin/agent-forge-core/issues/comments/6`; la réponse canonique est `404`. Le reçu expurgé `ordinary-issue-comment-delete-readback.json` contient uniquement `comment_id=6`, `http_status=404`, `verdict=absent`. Cela rend le refus fail-closed : tout statut autre que `404` est inconclusif ou indique que le commentaire existe encore.

Le webhook natif Forgejo `issue_comment` a ensuite émis `created`, `edited`, `deleted` pour ce même commentaire aux séquences `8`, `9`, `10`. Les trois livraisons ont été acceptées par le vrai receiver en HTTP `202`, reliées aux tâches Forgejo et réconciliées par le CLI. Le runner a aussi rejoué le scénario de doublon/restart préexistant; ce contrôle confirme la composition du banc, il n'est pas présenté comme nouvelle preuve de replay.

## Vérifications conservées

- `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e scripts.test_cargo_license_state -v` : **104 tests réussis**. Inclut le test du client API acceptant uniquement le `404` explicitement demandé, le refus d'un `403`, et le validateur qui refuse un `200` ou un ID invalide.
- `cargo fmt --all -- --check` : passe.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_e2e.py scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py` : passe.
- `git diff --check` : passe.
- `python3 scripts/verify_cargo_license_state.py` : passe; Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `NOASSERTION` et `redistribution_rights=not_assessed`. Aucun choix de licence produit n'a été fait.

`result.json`, `provenance.json`, `webhook-events.json`, `forgejo-hook-tasks.json`, `ordinary-issue-comment-delete-readback.json`, le readback de fermeture des ports, les logs et `SHA256SUMS` préservent les reçus. L'état privé du banc est sous `/tmp` et n'est pas copié dans la preuve.

## Limites

C'est une instance et un dépôt local, `n=1`, avec le binaire Forgejo cité mais sans provenance de compilation liée au commit audité. Un `404` au moment de la lecture prouve l'absence observée de cet ID à cet instant; il ne prouve ni une absence permanente, ni une garantie sur toutes les versions/routes. Le parcours utilise les outils de test locaux, sans AgentOS, RAG ou moteur mémoire externe, sans modèle, sans coût fournisseur et sans CI conventionnelle. Aucun gain général n'est établi.
