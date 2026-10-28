# FORGEJO-REVIEW-REQUEST-WEBHOOK-001 — run local 002

Date : 25 octobre 2026 (America/Montreal). Résultat : **réussi**, un run isolé (`n=1`).

## Portée et source

Ce lot étend le driver existant avec `--request-review`. Il demande réellement une revue à un utilisateur Forgejo de test dont l’accès au dépôt est en lecture, vérifie le HTTP 201 et la liste `requested_reviewers` par GET, puis relie l’événement et sa tâche Forgejo à l’enveloppe du receiver et au reconciler Rust. L’action `review_requested` et l’identité du reviewer servent à contrôler la liaison; elles ne sont pas une approbation ni un verdict de qualité.

Source Forgejo officielle v17.0.0 auditée au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Les fichiers et symboles contrôlés au HEAD épinglé avec `git show HEAD:<path>` :

- `modules/webhook/type.go::HookEventType.Event` : `pull_request_review_request` émet l’événement HTTP `pull_request`.
- `modules/structs/hook.go::HookIssueReviewRequested` et `PullRequestPayload.RequestedReviewer` : action `review_requested` et utilisateur dans le payload.
- `services/webhook/notifier.go::PullRequestReviewRequest` : construction de la livraison native.
- `routers/api/v1/repo/pull_review.go::CreateReviewRequests/apiReviewRequest` : API de demande, réponse 201.
- `services/issue/assignee.go::IsValidReviewRequest` : contrôle que le reviewer peut lire les pull requests.
- `services/convert/pull.go::ToAPIPullRequest` : readback des reviewers demandés.

Le scratch source `/tmp/forgejo-v17-base-cas-poc` contient d’autres changements de POC; seul le contenu épinglé de ces fichiers a été examiné ici. Le binaire de test est Forgejo `17.0.0+gitea-1.22.0`, Linux ARM64, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Son commit de build exact reste inconnu. Aucun code Forgejo n’a été copié dans le produit.

## Commande et observations

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root . \
  --state-dir /tmp/forgejo-review-request-webhook-002-state \
  --evidence-dir evidence/forgejo-review-request-webhook-002 \
  --request-review
```

Le résultat de la demande API est HTTP 201. Le readback contient le reviewer synthétique `forge-reader`, ID 2. Forgejo a enregistré huit tâches, toutes reçues par le receiver avec HTTP 202 : quatre événements d’issue et quatre de PR. Le groupe PR comprend `opened`, le commentaire de conversation, `review_requested` et `closed`. Le task type Forgejo `pull_request_review_request` est lié à l’événement HTTP `pull_request`, action `review_requested`, même UUID et même PR. Le payload de cet événement indique une PR ouverte; le reconciler Rust relit Forgejo et retourne `verdict=current_state_read`, PR #2, état final `closed`, `merged=false`, avec HEAD/base courants. La livraison fermée est distincte.

Le receiver a été tué par SIGKILL puis redémarré avec le même journal avant le replay synthétique : réponse duplicate acceptée, nombre d’entrées et SHA-256 du journal inchangés. Le hook est désactivé par readback; Forgejo et receiver sont arrêtés, leurs listeners absents. Le fixture ne sort pas de loopback et le candidat n’est pas exécuté.

La première tentative associée, [run 001](../forgejo-review-request-webhook-001/local-run.md), a échoué dans le nouveau harnais. Le dossier de preuve indique l’étape de demande; l’inspection de la fixture locale a confirmé que l’API avait créé le reviewer mais que le journal ne contenait pas encore la livraison. La cause était un contrat mal lu : `LocalForgejoApi.request` retourne `(body, headers)`, tandis que le harnais traitait `headers` comme le code HTTP. Le code a été corrigé pour lire `owner_api.last_status`; le présent run passe. Cette erreur était dans le harnais, pas un refus Forgejo.

## Vérifications et limites

- `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e` : **77/77** après réparation d’un test de trace qui se trouvait dans la classe CLI sans le `setUp` du parseur. Le premier résultat complet est gardé dans `driver-tests.log`, le vert dans `driver-tests-green.log`.
- Garde métadonnées de licence Cargo : **7/7**; `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`.
- Le driver E2E reconstruit les exécutables Rust offline/verrouillés sous Bubblewrap avec le réseau désactivé; voir [provenance](provenance.json) et [journal de build](cargo-build.log). `py_compile` et `git diff --check` passent sur l’état final.
- Le `provenance.json` de l’E2E conserve l’empreinte du fichier de tests avant le déplacement de ce test de classe, fait après le run. L’E2E ne lance pas ce module. L’empreinte du test final utilisé pour les 77/77 est consignée dans [post-run-test-source-sha256.txt](post-run-test-source-sha256.txt).
- Pièces principales : [résultat](result.json), [événements](webhook-events.json), [tâches Forgejo](forgejo-hook-tasks.json), [réconciliation de la demande](pull-request-review-request-event-reconciliation.json), [tests driver](driver-tests-green.log), [garde Cargo](cargo-license-guard-final.log), [diff-check final](diff-check-postdocs.log), [liens Markdown](relative-link-check.log), [cleanup](port-cleanup-readback.json), [manifest local](SHA256SUMS).

La source de produit n’a pas changé en Rust dans ce lot. Le protocole de demande de revue n’est pas une approbation, la lecture canonique est un contrôle de cohérence d’état et non un verdict indépendant du code. Une seule instance/utilisateur de test a été exercée; aucun LLM, AgentOS, RAG, mémoire, Actions, CI conventionnelle ou comparaison appariée n’a été utilisé. Aucun gain général ou baisse de coût n’est démontré.
