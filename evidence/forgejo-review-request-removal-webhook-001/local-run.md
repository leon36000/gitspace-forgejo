# FORGEJO-REVIEW-REQUEST-REMOVAL-WEBHOOK-001 — parcours local

Date : 25 octobre 2026 (America/Montreal). Résultat : **réussi**, un run isolé (`n=1`).

## Source et contrat observés

Forgejo source v17.0.0 est épinglé au commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`. `modules/structs/hook.go::HookIssueReviewRequestRemoved` définit l’action `review_request_removed`; `modules/structs/hook.go::PullRequestPayload.RequestedReviewer` porte l’identité du reviewer; `modules/webhook/type.go::HookEventType.Event` mappe `pull_request_review_request` à l’en-tête `pull_request`; `services/webhook/notifier.go::PullRequestReviewRequest` émet les deux actions selon le booléen de demande; `routers/api/v1/repo/pull_review.go::DeleteReviewRequests/apiReviewRequest` fournit DELETE et HTTP 204; `services/issue/assignee.go::ReviewRequest` appelle le chemin de retrait. Les extraits sont lus par `git show HEAD:<path>` parce que le scratch source contient d’autres POC; empreintes de contenu épinglé dans [upstream-source-hashes.txt](upstream-source-hashes.txt).

Le binaire Forgejo utilisé répond `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit exact de build reste inconnu. Aucun code Forgejo n’est copié dans le produit.

## Commande et résultat

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root . \
  --state-dir /tmp/forgejo-review-request-removal-webhook-001-state \
  --evidence-dir evidence/forgejo-review-request-removal-webhook-001 \
  --remove-review-request \
  --batch-pull-request-events
```

Le scénario utilise l’API locale pour ajouter `forge-reader` à la PR #2 (HTTP 201; readback `{id: 2, login: forge-reader}`), puis le retire (HTTP 204; readback des reviewers `[]`). Forgejo émet en ordre deux tâches `pull_request_review_request`, toutes deux liées à des UUID distincts et reçues par le receiver avec HTTP 202. Leurs payloads portent `pull_request` avec `review_requested`, puis `review_request_removed` et le même reviewer; la dernière livraison de PR ferme ensuite la PR sans merge. Au total, neuf tâches natives reçoivent HTTP 202.

Le vrai CLI Rust réconcilie les cinq deliveries PR dans une seule invocation batch. Les deux deliveries de revue sont séparément liées à des résultats `current_state_read` pour la PR #2; les résultats partagent l’état canonique courant `closed`/non fusionné. `upstream_read_attempts=1` est le compteur applicatif du batch, pas une capture HTTP indépendante. Le replay d’une livraison après SIGKILL du receiver conserve le nombre, les octets et le SHA-256 du journal. Readback confirme le hook désactivé, les processus arrêtés et les ports absents. Les commandes sont loopback; aucun code candidat n’est exécuté.

## Vérification et limites

- Driver Python : **79/79**; les cas ajoutés couvrent le parseur de retrait, le lien au reviewer et l’exposition CLI.
- Le E2E reconstruit `forgejo-webhook-local` et `forgejo-reconcile-local` avec Cargo `--offline --locked` sous Bubblewrap 0.9.0, réseau non partagé; voir [provenance](provenance.json) et [journal de build](cargo-build.log). La garde Cargo confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.
- Pièces : [résultat](result.json), [événements](webhook-events.json), [tâches Forgejo](forgejo-hook-tasks.json), [batch](pull-request-batch-reconciliation.json), [réconciliation de retrait](pull-request-review-request-removed-event-reconciliation.json), [replay](duplicate-delivery-replay.json), [cleanup](port-cleanup-readback.json), [manifest d’exécution](SHA256SUMS), [manifest complémentaire](SHA256SUMS.addendum), [tests](driver-tests.log) et [empreintes des sources](upstream-source-hashes.txt).

Les deux manifestes du dossier passent `sha256sum -c`. À la vérification finale, les index append-only ont zéro divergence parmi 5 198 chemins valides du worktree et 679 de l’index parent. Ils contiennent cependant des lignes historiques malformées laissées intactes : six dans le worktree (une entrée `None` visant un ancien test absent et cinq lignes vides) et une ligne vide dans l’index parent. Aucun fichier de ce lot n’est visé par ces lignes.

Le travail de ce lot modifie le runner/test Python, pas le noyau Rust. Il démontre un seul cycle sur une instance privée jetable et un seul binaire Forgejo dont le commit de build est inconnu. Il ne teste ni revue/approbation, annulation à mi-exécution, concurrence, LLM, AgentOS, RAG, mémoire, Actions, CI conventionnelle ou comparaison de valeur. Le retrait est un événement d’audit; il ne prouve pas la qualité du code ni l’autorité d’un reviewer.
