# CONTINUATION-316 — hausse concurrente de la règle d’approbation

Date : 27 octobre 2026.

## Question

Le chemin API normal de merge Forgejo vérifie-t-il encore le seuil courant d’approbation si une règle de protection est durcie après le retour du preflight et avant la fin du même POST merge?

## Source et environnement

- Source officielle Forgejo v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`.
- Expérience dans la nouvelle worktree détachée `/tmp/forgejo-v17-merge-review-race-001`, à ce commit. Le worktree principal `/tmp/forgejo-v17-base-cas-poc` est dirty et n’a pas servi au test.
- Le scratch ajoute une seule invocation de hook après le retour réussi de `pull_service.CheckPullMergeable` dans `routers/api/v1/repo/pull.go`; il ajoute aussi `routers/api/v1/repo/pull_merge_test_hook.go` et `tests/integration/api_pull_merge_race_test.go`. Le hook ne fait rien en dehors du test, et le test le libère après la mise à jour de règle.
- Test d’intégration Forgejo avec SQLite, serveur/route HTTP du harness et push Git local. Go `1.27.3 linux/arm64`; Git `2.43.0`; `CGO_ENABLED=1`. `git-lfs` n’est pas installé.

## Protocole et observation

Le test `TestAPIMergePullRequestCanUseProtectionBeforeConcurrentPolicyTightening` crée une protection de `master` exigeant 1 approbation, avec whitelist `user2`, `ApplyToAdmins=true` et `DismissStaleApprovals=true`. Il crée une PR et une revue officielle `APPROVED` de `user2`, liée au SHA de tête courant. Le compte `user1` envoie le chemin API normal `POST /pulls/{index}/merge` avec `Do=merge`, le `head_commit_id` exact, sans `force_merge`.

Après le succès de `CheckPullMergeable`, la barrière retient cette requête. Un autre appel API augmente le seuil à 2 et le readback confirme 2. Le test libère la requête originale : Forgejo retourne HTTP 200; le GET de la PR confirme `HasMerged=true`; un second readback confirme que la protection exige toujours 2 approbations. Le test est exécuté avec `-count=5` et repasse les cinq itérations. L’exécution ciblée avec `-race -count=1` réussit aussi.

Conclusion bornée : dans cette copie instrumentée et sur le chemin `Do=merge` exercé, la décision de protection n’est pas rebâtie à partir de la règle mise à jour entre le preflight et le merge. Cette observation confirme la course de mise à jour de règle; elle ne prouve pas le comportement de toutes les routes ni un impact sur une instance de production.

## Commandes et contrôles

Les commandes exactes sont dans `commands.txt`; leurs sorties finales sont conservées dans `integration-test-5x.log` et `integration-test-race.log`. `git diff --check` du scratch passe. Le précontrôle Make SQLite a été tenté et s’est arrêté parce que `git-lfs` manque; le test Go ciblé a été exécuté directement. Après amendement du reçu, `verify_manifests.py` vérifie le préfixe et les dernières empreintes du manifeste append-only; pour une vérification `sha256sum -c` sans anciennes versions de lignes dupliquées, utiliser le snapshot unique `SHA256SUMS.current`. Aucun autre test d’intégration ou suite complète n’est revendiqué.

## Tentatives et corrections de harnais

Les premières versions du test n’avaient pas une revue autorisée (422 pour l’auto-approbation), utilisaient un nom de fixture inexistant, puis un appel direct au setup de test sans serveur d’application (push refusé). Le test a été adapté aux utilisateurs de fixture autorisés et au helper d’intégration `onApplicationRun`. Ces essais de harnais ne sont pas inclus dans le compte de cinq passages finaux; les erreurs brutes n’ont pas été conservées dans ce reçu. L’absence de `git-lfs` est aussi un blocage de l’enveloppe Make, pas un échec du test Go ciblé.

Le fichier de test dans la worktree scratch a ensuite reçu le cas distinct CONT-317. Pour conserver l’input exact de ce run, son état 143 lignes et le SHA-256 `5109c7f24e412ec693caa9b3f5f72a8122fa83220fdd3408ae776e10eaaa2e21` sont préservés à `/tmp/forgejo-v17-merge-review-race-001-cont316-source-snapshot/api_pull_merge_race_test.go`.

## Limites

Le point d’arrêt est ajouté à une copie de source officielle; il rend la fenêtre déterministe mais n’est pas un retest du binaire amont intact. Un test isolé SQLite n’établit pas le comportement PostgreSQL, cluster, ni multi-instance. Seul l’endpoint API normal `Do=merge` est couvert. La révocation d’avis, les routes Web, `force_merge`, l’auto-merge, le chemin manuel, les autres writers de refs et les incidents de stockage ne sont pas testés. Aucun changement n’a été adopté dans Forgejo ou dans le code produit; aucun code source Forgejo n’a été copié dans le produit. Aucun coût externe, service externe, LLM ou secret n’a été utilisé.

Suite utile : placer la même barrière autour d’une révocation/dismiss d’approbation et lire les effets avant/après; ensuite examiner séparément chaque chemin de merge et déterminer si une politique verrouillée suffit au bac local ou si une précondition serveur commune aux writers est indispensable. Ce résultat n’autorise pas à adopter le patch CAS du scratch dirty.
