# CONTINUATION-321 — distinguer bypass admin et garde pre-receive

Date : 27 octobre 2026 (Montréal); journaux Go en UTC le 28 octobre. Source officielle Forgejo v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`.

## Pourquoi cette vérification

Après CONT-320, une lecture plus large a trouvé un contrôle après `CheckPullMergeable` dans `routers/private/hook_pre_receive.go::preReceiveBranch`. Les anciens tests CONT-316/317 créaient leur fusionneur comme `user1`; le Web CONT-318 utilise aussi `user1`. La fixture globale `models/fixtures/user.yml` identifie `user1` (ID 1) comme administrateur d’instance, tandis que `user2` (ID 2) n’est pas administrateur d’instance. Le premier test AutoMerge CONT-320 planifiait également sous `user1`. Cette différence de rôle pouvait expliquer le résultat.

Le code officiel de `preReceiveBranch` retourne dès que `ctx.user.IsAdmin`, avant `CheckPullBranchProtections`. Pour les autres utilisateurs, quand le push n’est pas directement autorisé et qu’un PR ID est présent, il appelle `CheckPullBranchProtections` au point du hook. Le merge normal construit son environnement avec `FullPushingEnvironment(..., pr.ID)`; le code du worker ne définit pas `GITEA_INTERNAL_PUSH`. Le CLI de hooks saute le pre-receive seulement lorsque cette variable vaut `true`. Les journaux du scénario non-admin confirment l’appel HTTP interne au hook et son refus 403.

## Test différentiel

Le test `TestAutoMergeWorker` couvre deux identités dans deux instances d’intégration isolées. Les deux scénarios créent une PR par `user1`, une protection `RequiredApprovals=1`, `ApprovalsWhitelistUsernames=[user2]`, `ApplyToAdmins=true` et `DismissStaleApprovals=true`; `user2` approuve la tête exacte. Le POST API normal planifie l’AutoMerge. Le corps réel du worker est ensuite suspendu après son `CheckPullMergeable` général, `user2` révoque l’avis via l’API, puis le worker reprend. Des readbacks Forgejo vérifient à la fois l’état PR/avis et le SHA Git de la branche de base.

- **Fusionneur administrateur d’instance (`user1`) :** le worker reprend et fusionne; PR `HasMerged=true`, avis `dismissed=true`, ref de base modifiée. Cette observation est compatible avec le bypass explicite d’instance-admin dans le pre-receive. Elle ne démontre pas une course générale pour les utilisateurs non-admins.
- **Fusionneur administrateur du dépôt seulement (`user2`) :** le worker reprend, mais le push atteint `POST /api/internal/hook/pre-receive/...`; le hook répond 403 avec `not enough approvals`. PR `HasMerged=false`, avis `dismissed=true`, ref de base byte-stable (même SHA avant/après). La politique `ApplyToAdmins=true` est active pour ce scénario.

Les deux tests passent 1/1; le groupe passe `-count=5` (les deux scénarios répétés cinq fois) et `-race -count=1`. Les commandes et journaux complets sont conservés ici. `gofmt -d` et `git diff --check` réussissent.

## Conclusion et portée

Le pre-receive constitue une seconde vérification utile lorsque l’acteur n’est pas administrateur d’instance, et refuse la révocation qui précède ce hook. En revanche, le chemin global-admin saute explicitement les protections dans ce hook; les résultats positifs CONT-316 à 320 utilisaient cette identité. Ils montrent le comportement pour un administrateur d’instance et rappellent de ne jamais donner ce credential à un agent; ils ne prouvent pas que tous les comptes peuvent franchir le hook après une mutation.

Le scénario non-admin ne prouve pas l’atomicité entre la lecture de policy/revue dans pre-receive et le déplacement Git de la ref. Une mutation après le contrôle du hook, mais avant la promotion du ref, reste à tester. Aucun test non-admin équivalent n’a encore été exécuté pour les routes API ou Web; leur push converge statiquement sur le même service `pull_service.Merge` et son hook, ce qui ne remplace pas une preuve directe. Tous les tests restent dans une copie instrumentée, SQLite, sans PostgreSQL, Actions ou production; le hook worker est direct via wrapper test-only, donc la queue AutoMerge n’est pas testée. Aucun correctif Forgejo n’est adopté.

Suite : dans le scratch seulement, ajouter une barrière après le `CheckPullBranchProtections` du pre-receive, puis révoquer l’approbation avant de laisser le hook retourner; observer si le ref peut encore bouger. Cette expérience répondra au dernier intervalle avant la mise à jour Git. Garder Forgejo comme socle inchangé en attendant cette preuve et l’estimation de tous les writers/coûts upstream.
