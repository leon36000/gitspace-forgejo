# FORGEJO-LIVE-ORDINARY-ISSUE-COMMENT-001 — commentaire d’issue natif jusqu’à la réconciliation

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, parent Git `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, avec de nombreuses modifications préexistantes. Aucun reset, commit, déploiement ni changement de production.

## Question et sources

Le trajet déjà testé avec des commentaires de conversation PR ne vérifiait pas le cas distinct d’un commentaire sur une issue ordinaire. Le source Forgejo v17.0.0 présent dans `/tmp/forgejo-v17-base-cas-poc` est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. `services/webhook/notifier.go::webhookNotifier.CreateIssueComment` choisit `HookEventIssueComment` lorsque `issue.IsPull` est faux et émet `IssueCommentPayload` avec `IsPull`; le cas PR choisit séparément `HookEventPullRequestComment`. La route API `POST /repos/{owner}/{repo}/issues/{index}/comments` est enregistrée dans `routers/api/v1/api.go` et traitée par `routers/api/v1/repo/issue_comment.go::CreateIssueComment`.

Le binaire local testé est `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Son lien de compilation exact avec le checkout officiel au commit ci-dessus n’est pas établi.

## Reproduction

```sh
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-ordinary-issue-comment-20261026-01 \
  --evidence-dir "$PWD/evidence/forgejo-live-ordinary-issue-comment-001" \
  --ordinary-issue-comment

python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v
python3 scripts/verify_cargo_license_state.py
cargo fmt --all -- --check
python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py
git diff --check
```

La commande E2E construit le receiver et le CLI Rust hors ligne avec `--locked` sous Bubblewrap 0.9.0, namespaces réseau/PID, environnement nettoyé et sources, registre Cargo et sysroot en lecture seule. Provenance, versions, SHA-256 des sources/binaires et politique de build : [`provenance.json`](provenance.json), [`cargo-build.log`](cargo-build.log). Journaux des vérifications complémentaires : [`python-tests.log`](python-tests.log), [`cargo-license-state.log`](cargo-license-state.log), [`source-checks.log`](source-checks.log).

## Résultat observé

Forgejo a créé une issue privée #1 puis une PR de fixture #2. Après les transitions précédentes, `POST /repos/forge-admin/agent-forge-core/issues/1/comments` a créé le commentaire #6. Forgejo a émis un événement réel `issue_comment` avec action `created`; le receiver Rust a répondu HTTP 202 et inscrit l’UUID `90edb639-b071-4a12-8abf-23afe3228da5` à la séquence 8. Le corps n’est pas renvoyé par le validateur du harness. Le payload indique `is_pull=false`, vise l’issue #1 et ne porte pas d’objet `pull_request`. La tâche webhook Forgejo associée a le même UUID, `event_type=issue_comment`, succès/délivré, HTTP 202.

Le vrai CLI `forgejo-reconcile-local` a consommé cette livraison et fait une lecture API de l’issue. [`ordinary-issue-comment-event-reconciliation.json`](ordinary-issue-comment-event-reconciliation.json) rapporte `current_state_read`, une lecture upstream, dépôt/numéro conformes et état canonique `closed`. La PR s’est terminée avant ce commentaire (séquence 7); cela confirme que le type d’événement reste celui d’une issue ordinaire malgré un dépôt contenant aussi une PR.

Le run complet compte huit livraisons Forgejo, toutes reliées par UUID aux tâches internes et reçues en 202; le hook est ensuite désactivé et relu inactif. Forgejo et receiver se sont arrêtés proprement, les deux ports attendus sont absents. Résultat structuré : [`result.json`](result.json); livraisons et readbacks : [`webhook-events.json`](webhook-events.json), [`forgejo-hook-tasks.json`](forgejo-hook-tasks.json), [`hook-readback.json`](hook-readback.json), [`port-cleanup-readback.json`](port-cleanup-readback.json).

Le module de tests Python du runner passe 90/90. `cargo fmt --all -- --check`, `py_compile`, `git diff --check` et le garde de licence passent. Cargo conserve `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`, et `NOASSERTION`.

## Portée et limites

C’est un run local n=1 avec une instance Forgejo jetable, un seul dépôt privé et un binaire identifié par hash; le commit exact de build du binaire n’est pas prouvé. La lecture des tâches de livraison utilise la table SQLite interne `hook_task` de Forgejo v17, non une API publique stable. Les processus partagent l’UID hôte. Aucun code candidat hostile n’est exécuté. Le run confirme l’émission native, le receiver, le journal et la réconciliation d’un événement; il ne démontre pas le parcours mission complet, l’intégration AgentOS/LLM, un gain comparatif ou un comportement en production.

Le dossier d’état local `/tmp/forgejo-live-ordinary-issue-comment-20261026-01` est conservé; il contient l’instance et des credentials/secrets synthétiques du test. Aucun secret n’est inclus dans les preuves persistées. Aucun service externe ni appel de modèle payant n’a été utilisé.
