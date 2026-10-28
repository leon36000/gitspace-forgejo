# FORGEJO-NATIVE-REJECTED-REVIEW-WEBHOOK-001 — résultat local

Date : 25 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; l’arbre était déjà largement modifié et ce lot conserve son état. La source Forgejo officielle dans `/tmp/forgejo-v17-base-cas-poc` est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`, mais le scratch reste fortement modifié par les POC sans rapport et n’a pas été touché par ce lot. Le binaire de test annonce Forgejo 17.0.0, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; son commit de build n’est pas établi.

## Parcours observé

Le scénario `--request-changes-review` du driver local crée un dépôt privé et une PR dans une instance Forgejo jetable. Un compte auteur non-admin distinct, limité à la permission `write` sur ce dépôt, ouvre la PR. Le propriétaire du dépôt crée ensuite une revue `REQUEST_CHANGES` liée explicitement au SHA de tête. La réponse de l’API Forgejo confirme `state=REQUEST_CHANGES`, l’auteur propriétaire attendu et ce SHA. Séparément, Forgejo émet l’événement `pull_request_rejected`; son task UUID correspond à l’UUID durable dans le journal, le receiver l’accepte en HTTP 202, et la relecture de la tâche confirme l’UUID, le type, le succès et le statut. Les événements contiennent `action=reviewed`, `review.type=pull_request_review_rejected`, le numéro de PR, et les SHA de base et de tête attendus.

Le scénario ferme ensuite la PR sans fusion. La réconciliation authentifiée de la livraison de rejet relit l’API Forgejo et retourne `current_state_read` avec l’état canonique final `closed`, `merged=false` et les SHA actuels. Le payload de la revue décrivait la PR ouverte avant sa fermeture; il sert donc de signal de relecture, pas de vérité d’état, d’approbation ni de preuve que le code est correct. Le lecteur API demeure un compte non-admin en permission `read`, avec des scopes de lecture. Le harnais vérifie aussi le replay de la première livraison après redémarrage du receiver et l’intégrité du journal.

Le run a reçu huit tâches Forgejo et huit événements; une tâche était de type `pull_request_review_rejected`. Les résultats et readbacks sont dans `result.json`, `webhook-events.json`, `forgejo-hook-tasks.json` et `pull-request-rejected-review-event-reconciliation.json`. Le rapport lie le scénario au vrai receiver et au vrai CLI local; il ne démontre pas un agent, une décision de fusion ou un jugement de qualité.

## Vérifications reproductibles

Commande du parcours live :

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root . --state-dir /tmp/forgejo-native-rejected-review-001-state --evidence-dir evidence/forgejo-native-rejected-review-webhook-001 --request-changes-review
```

Le run a réussi (`verdict=passed`). La suite du driver Python donne 68/68 tests réussis; `py_compile` passe pour le driver et son module de test. Les commandes et versions, les hashes de sources, les artefacts, la configuration de hook expurgée, la lecture de permission et la preuve de nettoyage sont conservés dans ce dossier. `SHA256SUMS.final` couvre les fichiers du dossier, y compris les ajouts postérieurs au manifeste initial du runner. Forgejo et le receiver ont été arrêtés, la hook désactivée par API avant arrêt, et aucun listener des deux ports ne subsiste. La suite Python n’appelle pas de service externe.

Le contrat API vérifié dans le dépôt officiel au commit indiqué est `routers/api/v1/repo/pull_review.go` : `CreatePullReview` prépare le type, lit ou reçoit `commit_id` et soumet la revue (lignes 431–547); `preparePullReviewType` rejette les revues de sa propre PR et mappe `REQUEST_CHANGES` au type `ReviewTypeReject` (lignes 644–685). Le test local ajoute l’acceptation positive du type rejeté et refuse le type approuvé substitué.

## Limites

`n=1`, Forgejo local jetable; le build binaire n’est pas rattaché à un commit source exact. Le compteur de GET upstream vient du résultat applicatif du CLI, sans observateur HTTP indépendant dans ce run. Cette tranche ne modifie pas le code Rust produit et ne mesure pas le comportement d’un LLM, d’AgentOS, du RAG ou d’une mémoire, ni un benchmark comparatif. Le code Forgejo, le code GitSpace et tout autre dépôt tiers n’ont pas été copiés dans le produit. La licence du produit demeure non choisie et les droits de redistribution non évalués. Le projet reste incomplet.
