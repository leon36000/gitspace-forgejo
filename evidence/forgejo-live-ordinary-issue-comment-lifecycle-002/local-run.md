# FORGEJO-LIVE-ORDINARY-ISSUE-COMMENT-LIFECYCLE-002 — création, édition et suppression réelles

Date : 26 octobre 2026. Résultat : **réussi sur une instance Forgejo jetable locale, n=1**.

## Source examinée

Le checkout scratch de Forgejo est au commit officiel `v17.0.0`, `0d791421f85ca13a63b665053d81ea765fbbc571`. Les routes et notifications ciblées sont dans `services/webhook/notifier.go` (`UpdateComment`, `CreateIssueComment`, `DeleteComment`), `routers/api/v1/api.go` (routes d'issues/commentaires) et `routers/api/v1/repo/issue_comment.go` (`CreateIssueComment`, `EditIssueComment`, `DeleteIssueComment`). Ces trois fichiers n'avaient pas de modifications locales au moment du contrôle; leurs SHA-256 sont consignés dans la session de travail. L'examen se limite à ces symboles et routes.

Le binaire local `/tmp/forgejo-17.0.0` annonce `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. **Son lien de compilation avec le commit source audité n'est pas établi.** Aucun code Forgejo n'a été copié ou adopté.

## Parcours exécuté

```sh
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-live-ordinary-issue-comment-lifecycle-002 \
  --evidence-dir evidence/forgejo-live-ordinary-issue-comment-lifecycle-002 \
  --ordinary-issue-comment \
  --edit-ordinary-issue-comment \
  --delete-ordinary-issue-comment
```

Dans un dépôt privé synthétique sur `127.0.0.1`, le runner crée le commentaire ordinaire #6 de l'issue #1, modifie puis supprime **le même ID**. Les réponses Forgejo passent les contrôles du runner pour création `201`, édition `200`, suppression `204`. Forgejo émet trois événements `issue_comment` `created`, `edited`, `deleted`, aux séquences 8, 9 et 10 avec des UUID distincts. Le récepteur réel répond `202` pour chacune; les trois tâches internes Forgejo ont le type `issue_comment`, `is_succeed=true`, et sont liées aux livraisons reçues. Le parseur n'expose ni texte de commentaire ni `changes` non fiable. Le vrai CLI réconcilie chaque événement en relisant l'issue actuelle une fois; chaque résultat est `current_state_read`, et l'état canonique reste `closed`.

Le scénario standard inclus vérifie aussi le rejeu d'une livraison après redémarrage forcé du récepteur : le journal conserve ses 10 entrées et ses octets/hash avant/après. Le hook est désactivé puis relu inactif. Forgejo et le récepteur sont arrêtés; le readback constate leurs ports sans écouteur. Les jetons synthétiques restent dans `/tmp` (répertoire privé hors preuve); aucun secret n'est consigné.

Artefacts : `result.json`, `webhook-events.json`, `forgejo-hook-tasks.json`, trois réconciliations commentaire, `provenance.json`, `port-cleanup-readback.json`, journaux de vérification et `SHA256SUMS` local.

## Échec du harnais et correction

Une première tentative conservée dans `../forgejo-live-ordinary-issue-comment-lifecycle-001/` a accompli les mutations et reçu les livraisons, mais a échoué au contrôle `read_forgejo_delivery_tasks`: le validateur associait encore les tâches à un événement commentaire scalaire (`created`) et omettait `edited`/`deleted`. La lecture Forgejo contenait 10 tâches alors que le vecteur transmis au validateur en avait moins. Le runner collecte maintenant la séquence complète, lie chacune des trois tâches à son UUID et valide l'ordre. L'instance du premier essai a été arrêtée et son hook désactivé; son échec et ses reçus sont conservés, sans être présenté comme un succès.

Après correction, les tests du module du runner passent `92/92` et le parcours isolé ci-dessus passe. Le hash du runner utilisé figure dans `provenance.json` (`34494a74049c0fea957ff9bfa0444279e2f66ea5451cdf846e4c361087881a9a`); les tests sont également hachés dans la provenance. `cargo fmt --all -- --check`, `py_compile`, `git diff --check` et la garde d'état de licence Cargo passent.

## Limites

Ce parcours prouve l'émission réelle de ces trois sous-types par **ce binaire local**, leur réception et la relecture de l'état canonique actuel. Il ne prouve pas que le binaire a été construit depuis le commit audité, ni la compatibilité de toutes les versions Forgejo. Le `204` de suppression est vérifié et l'événement `deleted` reçu; aucun GET post-suppression séparé ne vérifie l'absence du commentaire. L'issue ne change pas d'état au cours des mutations de commentaire; la réconciliation n'interprète pas le corps comme vérité métier.

Ce n'est ni un parcours avec AgentOS/LLM, ni une comparaison de CI conventionnelle, ni une mesure de gain. Aucun appel modèle, coût, service externe, production ou publication. Le projet et la comparaison restent incomplets.
