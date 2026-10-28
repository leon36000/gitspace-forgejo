# CONTINUATION-233 — rapprochement de la matrice de refus workflow

Date : 27 octobre 2026. Travail documentaire dans le worktree local fortement modifié, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun code produit n’a changé et aucun scénario n’a été rejoué.

## Mise à jour

La matrice canonique `docs/refusal-evidence-matrix.md` omettait une série récente de refus portant sur les actions `workflow_run` et `workflow_job`. Une ligne les rattache désormais aux contrats `SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS`, au refus HTTP du vrai receiver avant append et au refus du vrai CLI pour une action incompatible déjà persistée. Les rapports liés sont CONT-229, CONT-230, CONT-231 et CONT-232.

La ligne conserve les limites observées : l’audit de contrat est ciblé sur Forgejo v17.0.0; aucun Forgejo live ni runner Actions n’est impliqué dans ces essais; les lignes replayées sont des fixtures dont le hash est recalculé et ne préservent pas l’authenticité HMAC; aucune couverture LLM/AgentOS ou valeur comparative n’en découle. Les rapports et leurs commandes/logs précédents restent la preuve d’exécution; ce lot ne les reformule pas en nouveaux runs.

## Vérification

Les quatre rapports référencés sont présents. Le manifeste local CONT-232 vérifie ses onze fichiers; ses entrées récentes sont inscrites dans les index append-only du worktree et du projet, dont les préfixes antérieurs sont conservés. `git diff --check` passe sur le diff documentaire courant. Aucune suite logicielle ou intégration Forgejo n’a été exécutée.

Les commandes, versions, sorties et empreintes du présent rapprochement sont conservées dans ce dossier. Cette mise à jour améliore la traçabilité; elle ne change pas l’état produit ni les limites cumulatives. Le projet demeure incomplet.
