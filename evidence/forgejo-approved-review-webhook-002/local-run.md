# FORGEJO-APPROVED-REVIEW-WEBHOOK-002 — parcours local

Date : 25 octobre 2026 (America/Montreal). Résultat : **réussi**, un run (`n=1`).

## Source et contrat observés

Forgejo officiel v17.0.0 est épinglé à `0d791421f85ca13a63b665053d81ea765fbbc571`. Les blobs de `routers/api/v1/repo/pull_review.go`, `routers/api/v1/utils/hook.go`, `models/webhook/webhook.go`, `services/webhook/notifier.go`, `modules/webhook/type.go` et `modules/structs/hook.go` sont hachés depuis `git show HEAD:<path>` dans [upstream-source-hashes.txt](upstream-source-hashes.txt). Le scratch source `/tmp/forgejo-v17-base-cas-poc` contient des POC non reliés à ce parcours; ses fichiers de travail ne servent pas de source d’autorité.

Dans ce code épinglé, `CreatePullReview` appelle `preparePullReviewType`; l’état `APPROVED` devient `ReviewTypeApprove`, et Forgejo refuse qu’un auteur approuve sa propre PR. Le notifier émet l’événement interne `pull_request_review_approved`; le header wire est `pull_request_approved`, tandis que le payload porte `action=reviewed` et `review.type=pull_request_review_approved`. Le commit de build exact du binaire local reste inconnu.

## Commande et résultat

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root . \
  --state-dir /tmp/forgejo-approved-review-webhook-002-state \
  --evidence-dir evidence/forgejo-approved-review-webhook-002 \
  --approve-review \
  --batch-pull-request-events
```

Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, tourne seulement sur `127.0.0.1:3130`. Un auteur distinct, `forge-review-author` (non-admin, écriture dans le dépôt privé), ouvre la PR #2. L’utilisateur `forge-admin` soumet ensuite l’approbation : le POST API répond HTTP 200, le readback renvoie `state=APPROVED` et le SHA candidat `ee1b685669f5cebbb9cfd4422c56bdc755cd885c`.

Le webhook natif reçu a le header `pull_request_approved`, l’action `reviewed`, `review.type=pull_request_review_approved`, le même SHA de tête, la base `4070e3034e7c929102de9ecab946153dbe1e7f7d`, et la PR encore ouverte/non fusionnée au moment de la revue. Son UUID `aac71551-c6eb-4fc1-8d72-56a2ddfe6fcb` correspond au task Forgejo qui reçoit HTTP 202 et à l’entrée durable du journal. Le batch réconcilie quatre événements PR (`opened`, commentaire de conversation, approbation, puis fermeture non fusionnée) dans une invocation; le CLI déclare `upstream_read_attempts=1` et retourne quatre états canoniques. L’événement d’approbation est réconcilié en `current_state_read` avec l’état courant final `closed`/non fusionné : le verdict d’approbation dans le webhook n’est pas utilisé comme état canonique.

Au total, huit tâches webhook natives reçoivent HTTP 202. Le replay après SIGKILL du receiver conserve huit entrées, la longueur et l’empreinte du journal. Le hook est désactivé; Forgejo, le receiver et leurs listeners sont arrêtés. Readbacks dans [result.json](result.json), [événements](webhook-events.json), [batch](pull-request-batch-reconciliation.json), [réconciliation de l’approbation](pull-request-approved-review-event-reconciliation.json), [tâches](forgejo-hook-tasks.json), [replay](duplicate-delivery-replay.json) et [cleanup](port-cleanup-readback.json).

## Vérification et limites

- Driver Python : **81/81**; inclut l’exposition CLI et le refus de combiner les scénarios `APPROVED` et `REQUEST_CHANGES`.
- Licence Cargo : **7/7** tests. Readback : `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`. Aucun identifiant de licence ou droit de redistribution n’est choisi.
- Python compile; `git diff --check` est propre. Les exécutables Rust utilisés par le parcours sont reconstruits avec Cargo `--offline --locked` sous Bubblewrap 0.9.0, avec namespaces réseau et PID isolés; Rust/Cargo 1.95.0. Les reçus sont dans [provenance](provenance.json) et [cargo-build.log](cargo-build.log).
- Les empreintes Forgejo citées sont celles des blobs amont épinglés, pas celles d’un checkout propre : le scratch est expérimental et le commit du binaire exécuté n’est pas établi. Aucun code Forgejo n’est copié au produit. Ce lot ajoute le scénario dans le driver/test Python; il ne modifie pas le noyau Rust.
- Portée : un run local SQLite, un reviewer propriétaire distinct de l’auteur, une PR close non fusionnée. Cela n’évalue ni tous les rôles/politiques de revue, ni une approbation exploitable comme autorité de merge, ni un parcours LLM/AgentOS, ni une CI conventionnelle ou une comparaison de coût/temps. Aucune amélioration générale n’est inférée.

Les deux manifestes propres au dossier passent `sha256sum -c`. L’audit final des index append-only trouve zéro divergence parmi 5 226 chemins valides du worktree et 709 de l’index parent; six et une lignes historiques malformées sont conservées (dont une entrée `None` pointant vers un ancien test absent). Aucune ne vise ce lot. Aucun secret de webhook/API n’est conservé dans les artefacts.
