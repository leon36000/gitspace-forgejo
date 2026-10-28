# CONTINUATION-323 — course post-check via la route Web

Date du lot : 27 octobre 2026, Montréal; journaux en UTC le 28 octobre. Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Scratch : `/tmp/forgejo-v17-merge-review-race-001`, checkout détaché mais dirty de seams/tests uniquement.

## Scénario

Le test `TestWebMergePullRequestCanUseApprovalBeforeConcurrentDismissalDuringPreReceive` reprend la même barrière test-only que CONT322, après le retour réussi de `CheckPullBranchProtections` dans `preReceiveBranch`. Cette fois le merge initial est un POST Web authentifié par la session de `user2`, propriétaire/admin du dépôt mais non admin global selon la fixture amont. `user1` crée la PR et, via l’API, révoque l’avis officiel exact pendant que le vrai endpoint HTTP interne pre-receive est arrêté.

Protection : une approbation requise, reviewer whitelist `user2`, `DismissStaleApprovals=true`, `ApplyToAdmins=true`. Le test lit la tête exacte et le SHA de base avant la requête, puis observe les readbacks après.

## Résultat

Le hook interne retourne 200; le POST Web `/{owner}/{repo}/pulls/{index}/merge` retourne 200. L’API de lecture confirme `HasMerged=true`, la revue reste `dismissed=true`, et la branche de base a un nouveau SHA. L’API et le Web sont donc tous deux observés acceptant une révocation effectuée après leur contrôle pre-receive, dans des scénarios instrumentés séparés.

Le test passe 1x (1,291 s dans le log archivé), `-count=5` (6,163 s), et `-race -count=1` (9,357 s). `gofmt -d` et `git diff --check` sont propres.

## Source et portée

Dans le blob amont `routers/web/repo/pull.go::MergePullRequest`, la voie normale appelle `CheckPullMergeable`, puis `pull_service.Merge`. `services/pull/merge.go::doMergeAndPush` crée le push avec le PR ID. Le scratch ajoute un callback de test après le check dans le hook privé; ce callback est gardé par `testing.Testing()` et absent du blob amont. Les empreintes exactes sont dans `source-sha256.txt`; les lignes et séquence sont dans `source-review.txt`.

Limites : SQLite et scratch instrumenté, une fixture, une route Web, pas un binaire upstream intact, PostgreSQL, production, Actions, worker AutoMerge via queue, force/manual ou tous les writers. La révocation est faite par `user1`; cette expérience ne montre pas que l’agent a cette capacité. Elle ne justifie pas seule une modification de Forgejo; il faut cartographier les writers communs et le coût de maintenance/rebase avant toute décision.

Le projet de forge demeure incomplet; aucun gain comparatif n’est mesuré. Aucun code Forgejo n’est copié ou adopté au produit.

## Reproduction

Les commandes sont dans `commands.txt`, les versions dans `tool-versions.txt`, les sorties dans `integration-test-*.log`, et les sources/hashes dans `source-review.txt` et `source-sha256.txt`.
