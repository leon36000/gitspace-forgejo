# WEBHOOK-LIVE-MISSION-ADMISSION-006 — attente du résultat terminal de Forgejo

Date : 24 octobre 2026. Exécution sur le worktree isolé, Forgejo local uniquement.

## Résultat

Une relecture indépendante du driver 005 a trouvé une course dans `_wait_for_issue_task` : il rendait la tâche dès que `is_delivered=true`, alors que le code Forgejo audité peut marquer la tâche avant l’appel HTTP. La validation exige ensuite `is_succeed=true` et HTTP 202. Le test rouge a reproduit le retour prématuré sur l’état transitoire `delivered=true, succeeded=false, status=null`.

Le driver attend maintenant soit le succès complet HTTP 202, soit une réponse HTTP terminale non-202 qui est transmise au validateur pour refus. Il ne transforme pas un statut en succès. Un test séparé confirme que l’échec HTTP 401 est remis immédiatement au validateur; le cas d’expiration sans réponse reste borné à dix secondes.

Le scénario a été rejoué avec Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, dans `/tmp/forgejo-live-mission-webhook-009-20261024-racefix`. Forgejo a émis une tâche `issues/opened`; son UUID `48cc4add-0c51-469b-a5a4-2b58681d9dca` correspond au delivery du receiver, séquence 1, HTTP 202. L’admission a épinglé la base `025240efeaefbddd1b0c491a72706e6fb0a5c740` et l’exigence `MISSION-012-CLAMP-001` SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`.

Le credential hôte invalide a été refusé (`mission_source_unauthorized`) avant création du journal ou de l’admission. Le rejeu signé a rendu `already_claimed`, sans modifier leurs empreintes et avec une seule entrée inbox. Forgejo confirme la désactivation du hook; le serveur et le receiver ont quitté avec code 0 et aucun listener contrôlé n’est resté.

## Vérifications et provenance

- Le test de régression a échoué avant le correctif sur le premier readback transitoire.
- Tests après correctif : 9/9 pour `test_forgejo_live_webhook_mission_admission.py`; 6/6 pour `test_forgejo_emitted_webhook_e2e.py`.
- Le contrôle Cargo confirme `license=null`, `license_file=null`, `publish=[]`; décision de licence `unselected`, droits de redistribution `not_assessed`.
- Analyse syntaxique et espaces finaux : OK. `git diff --check` : OK.
- Les 47 empreintes source dans `provenance.json` correspondent aux fichiers source utilisés pour ce run. Le commit HEAD examiné reste `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; Forgejo v17 est identifié par version et hash de binaire, sans preuve que ce binaire a été construit depuis un commit source particulier.
- Le checkout source Forgejo relu pour la sémantique de livraison est à `ea9ae1e174ac290febcd92f57e7a28ccb58262d0`, symbole `services/webhook/deliver.go::Deliver` : `MarkTaskDelivered` précède l’envoi HTTP. Cette lecture explique la course, mais n’établit pas la provenance de construction du binaire local.
- `provenance.json` décrit maintenant explicitement que le **contenu** privé de l’état n’est pas inclus. Les artefacts d’admission exposent les chemins absolus de certains fichiers sous `/tmp`; la formulation historique trop large du run 005 est documentée dans `provenance-review-005.json`.
- Le manifeste de ce run couvre tous les artefacts, y compris ce rapport et les journaux de tests et contrôles.

## Limites

Ce run s’arrête encore à l’admission : aucun agent, modification de code, PR, vérification de candidat ou merge n’a été exécuté. Il n’a pas forcé la course sur le worker HTTP live; le scénario réel a confirmé le succès et le test unitaire a imposé l’état transitoire. Les deux preuves établissent des étapes distinctes.

Un parcours complet antérieur a utilisé Claude Code Opus 5.5/high, MCP, vérification indépendante et merge en bac à sable dans `mission-012-live-agent-webhook-compose-005`. Son admission envoyait une requête HMAC signée directement au receiver; elle ne prouve pas que Forgejo lui-même a émis le webhook. Ainsi, l’émission Forgejo réelle et le parcours agent/PR réussi ne sont toujours pas démontrés dans une seule exécution intégrée. Aucun nouvel appel de modèle n’a été lancé ici.

Les services locaux partagent toujours l’UID hôte; la lecture SQLite `hook_task` n’est pas une API publique Forgejo. Aucun résultat de productivité, de coût comparatif ou d’usage réel n’est revendiqué. L’état privé `/tmp/forgejo-live-mission-webhook-009-20261024-racefix` est conservé et n’a pas été supprimé.

