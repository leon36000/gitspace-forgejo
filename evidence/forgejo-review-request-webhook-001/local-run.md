# FORGEJO-REVIEW-REQUEST-WEBHOOK-001 — run local 001 (échec du harnais)

Date : 25 octobre 2026 (America/Montreal). Résultat : **échec d’instrumentation**, pas un succès produit.

La première exécution de `--request-review` a créé une demande de revue dans le Forgejo jetable, mais le runner s’est arrêté à l’étape `deliver_pull_request_review_request_event`; `failure.json` le consigne. Le journal reçu s’arrêtait aux événements antérieurs et aucune preuve de livraison/reconciliation n’a été retenue. Le hook est ensuite désactivé par readback, Forgejo et receiver arrêtés, listeners absents (`port-cleanup-readback.json`). La configuration du hook contenait l’événement requis.

Cause établie par le chemin d’appel : [run_forgejo_live_webhook_reconcile_e2e.py](../../scripts/run_forgejo_live_webhook_reconcile_e2e.py) définit `LocalForgejoApi.request` pour retourner `(body, headers)` et stocke le code entier dans `last_status`. Le nouveau scénario avait affecté le dictionnaire d’en-têtes à `review_request_http_status`, puis comparé ce dictionnaire à `201`; il a donc rejeté son propre readback malgré la création de la demande. Le correctif lit maintenant `owner_api.last_status`. Le run corrigé et réussi est [FORGEJO-REVIEW-REQUEST-WEBHOOK-001 run 002](../forgejo-review-request-webhook-002/local-run.md).

Forgejo de test : binaire v17.0.0+gitea-1.22.0, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, commit exact de build inconnu. L’état privé, dont le secret synthétique du hook, reste hors de ce dossier de preuve. Aucun LLM ni service externe utilisé.
