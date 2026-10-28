# FORGEJO-NATIVE-PR-REVIEW-COMMENT-WEBHOOK-001 — résultat local

Date : 25 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre déjà fortement modifié. Aucun commit ni staging effectué. Sources et exécutables utilisés : voir `provenance.json` et `source-sha256-final.txt`.

## Résultat

Le driver expose `--pull-request-review-comment` et envoie à Forgejo une vraie revue `COMMENT` avec une note inline sur `webhook-e2e.txt`, attachée au SHA candidat. L’API rend un état `COMMENT` et le SHA demandé; Forgejo émet son événement natif avec l’action `reviewed`, le type `pull_request_review_comment` et le même head/base de PR. La tâche webhook Forgejo correspondante a reçu HTTP 202 et est liée à l’UUID de livraison journalisé.

La composition activée a traversé dix tâches émises par Forgejo : quatre événements d’issue, puis ouverture et synchronisation de la PR, commentaire de conversation, commentaire inline de revue, revue `REQUEST_CHANGES` et fermeture non fusionnée. Les six événements PR ont été transmis au reconciler Rust dans l’ordre du journal en une seule invocation batch. Les six résultats sont liés à leur delivery et à la PR #2, et le batch rapporte `current_state_read` avec `upstream_read_attempts=1`. Ce compteur est la télémétrie du CLI; aucune capture HTTP indépendante de cette lecture n’a été faite. Le readback propriétaire confirme `closed`, `merged=false`, head/base attendus. Le replay après SIGKILL du récepteur n’a pas changé la taille ni le SHA du journal. La hook a été désactivée par readback; les deux processus sont arrêtés et aucun listener du fixture ne subsiste.

Commande exécutée :

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-native-pr-review-comment-001-state --evidence-dir evidence/forgejo-native-pr-review-comment-webhook-001 --synchronize-pull-request --request-changes-review --pull-request-review-comment --batch-pull-request-events
```

## Références et vérifications

- Forgejo officiel v17.0.0, source auditée dans le scratch au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. `routers/api/v1/repo/pull_review.go::CreatePullReview` crée les commentaires de code fournis par l’API avant de soumettre la revue; `services/webhook/notifier.go::PullRequestReview` choisit `HookEventPullRequestReviewComment` pour une revue de type commentaire; `modules/structs/pull_review.go::CreatePullReviewComment` définit `new_position`. La correspondance wire `pull_request_comment` / `review.type=pull_request_review_comment` est aussi documentée dans [l’audit du contrat webhook](../forgejo-comment-wire-001/local-run.md). Ces fichiers ciblés sont propres au commit source épinglé; d’autres fichiers du scratch Forgejo sont modifiés par des POC et le commit de build du binaire n’est pas établi.
- Binaire local : Forgejo `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.
- Le driver a reconstruit les binaires Rust via Cargo `1.95.0` / rustc `1.95.0`, Bubblewrap `0.9.0`, `--offline --locked`, `--unshare-net` et `--unshare-pid`. Les empreintes des exécutables et des sources d’exécution figurent dans `provenance.json`.
- `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e` : 71/71; `py_compile` : succès.
- Garde Cargo : 7/7; `license=null`, `license_file=null`, `publish=[]`, choix `no_license_selected`, droits `not_assessed`.
- `git diff --check` : succès. Pas de changement Rust; la suite Rust complète n’a pas été relancée.

## Limites

Un run local (`n=1`) vérifie le câblage de l’événement natif et sa réconciliation, pas une amélioration de productivité ni un coût réduit. Aucun LLM, AgentOS, RAG, mémoire, CI conventionnelle, service externe ou benchmark n’a été utilisé. Forgejo, le récepteur et le CLI tournent sous le même compte hôte; aucun code candidat hostile n’est exécuté. Le résultat ne démontre pas une fusion de cette PR (une composition de fusion antérieure est documentée séparément). Aucun code Forgejo ni GitSpace n’a été copié au produit et aucune licence produit n’a été choisie.

Suite locale : poursuivre une lacune d’intégration distincte dans le scratch Forgejo ou une capacité de reprise/récupération encore non couverte, sans refaire les parcours PR déjà démontrés. Garder le patch Forgejo non adopté et le projet marqué incomplet tant que le parcours LLM et la comparaison contrôlée avec agent/CI conventionnels ne sont pas exécutés avec un plafond autorisé et fiable.
