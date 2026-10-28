# CONTINUATION-322 — révocation après le contrôle pre-receive

Date du lot : 27 octobre 2026, Montréal; horodatages des journaux en UTC le 28 octobre. Référence amont : Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Scratch : `/tmp/forgejo-v17-merge-review-race-001`, detached à ce commit mais dirty de seams/tests seulement; aucun fichier Forgejo n’a été copié dans le produit.

## Question testée

CONT321 avait montré qu’un compte non-global-admin est refusé lorsque la révocation d’avis précède son contrôle dans `preReceiveBranch`. Ici, une barrière test-only est placée immédiatement après la réussite de `CheckPullBranchProtections`, avant que le hook pre-receive retourne au processus Git. Le but est de vérifier si une révocation faite dans cette fenêtre empêche encore le déplacement du ref.

## Scénario et résultat

Le test d’intégration `TestAPIMergePullRequestCanUseApprovalBeforeConcurrentDismissalDuringPreReceive` utilise la fixture SQLite du dépôt `user2/repo1`. La protection exige une approbation et autorise l’avis de `user2`, applique la règle aux administrateurs et rejette les approbations devenues obsolètes. `user1` crée la PR; `user2`, administrateur du dépôt mais non administrateur global, l’approuve sur son SHA de tête exact et en demande le merge via le POST API normal. `user1` révoque cet avis après l’arrêt du vrai endpoint HTTP pre-receive.

Après libération, le hook retourne HTTP 200 et l’API merge retourne HTTP 200. Les readbacks Forgejo constatent `HasMerged=true`, la revue `dismissed=true` et un SHA de branche de base différent de celui capturé avant le merge. Le journal d’intégration confirme le POST à `/api/internal/hook/pre-receive/user2/repo1`, le dismissal HTTP 200, puis le POST normal `/api/v1/repos/user2/repo1/pulls/6/merge` HTTP 200. Ce résultat démontre une interleaving check→write acceptée pour cette voie API et cette copie instrumentée.

Le test cible passe une fois (1,622 s dans le log archivé), `-count=5` passe (6,161 s), et `-race -count=1` passe (9,401 s). `gofmt -d` sur les deux sources scratch est vide; `git diff --check` est propre.

## Sources examinées

- Forgejo officiel `routers/private/hook_pre_receive.go::preReceiveBranch` : bypass global-admin aux lignes 457–460; `CheckPullBranchProtections` pour l’acteur non-global-admin aux lignes 472–490. L’amont ne contient aucun seam de test.
- `routers/api/v1/repo/pull.go::MergePullRequest` : `CheckPullMergeable`, puis appel normal à `pull_service.Merge`.
- `services/pull/merge.go::doMergeAndPush` : construit un environnement de push avec le PR ID; le hook reçoit donc l’identité de la PR.
- Le scratch ajoute seulement un callback sous `testing.Testing()` après le check réussi, puis un test qui pilote l’API et le hook interne. Voir les hashes et lignes exacts dans `source-review.txt` et `source-sha256.txt`.

## Limites

Cette preuve est un test d’intégration SQLite sur une copie du code amont instrumentée par des seams test-only. Le passage par l’API merge et le endpoint HTTP interne pre-receive est réel pour cette fixture, mais cela ne vérifie pas un binaire upstream intact, PostgreSQL, déploiement, cluster ou production. Elle couvre un acteur repository-admin non-global et une route API seulement; elle ne teste pas le chemin Web, le dispatch de queue AutoMerge, force/manual, Actions, ni tous les writers de refs/policy. Elle ne montre pas qu’un agent puisse révoquer l’avis ou obtenir le credential de fusion.

Conséquence limitée : le contrôle pre-receive observé n’est pas atomique avec le déplacement du ref quand l’avis change après ce contrôle. Aucun patch Forgejo n’est adopté; le coût et la compatibilité d’une clôture commune restent à mesurer après inventaire des writers. Le projet global demeure incomplet et aucun gain comparatif n’est établi.

## Reproduction

Les commandes déterministes sont dans `commands.txt`, les versions dans `tool-versions.txt`, les hashes dans `source-sha256.txt` et les sorties dans `integration-test-*.log`. La revue complémentaire et les essais de harnais sont dans `source-review.txt` et `attempts.txt`.
