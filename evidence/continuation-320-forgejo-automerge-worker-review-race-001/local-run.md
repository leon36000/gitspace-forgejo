# CONTINUATION-320 — course de révocation dans le worker AutoMerge Forgejo

Date : 27 octobre 2026, heure locale Montréal; les journaux du run sont horodatés en UTC le 28 octobre. Source amont examinée : Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`.

## Question

Le worker AutoMerge peut-il fusionner une PR après que son dernier `CheckPullMergeable` a accepté l’approbation requise et qu’un administrateur a ensuite révoqué cette approbation?

## Méthode et état du scratch

Le test tourne dans `/tmp/forgejo-v17-merge-review-race-001`, détaché au commit officiel ci-dessus. Cette worktree n’est pas vierge : elle contient les hooks et tests test-only des lots CONT-316 à 318, plus le hook de test AutoMerge de ce lot. Aucun de ces changements n’est copié dans le dépôt produit, et aucun code Forgejo n’est adopté.

Le test configure une protection SQLite qui exige une approbation, autorise `user2` comme réviseur et active la révocation des approbations périmées. Il crée une PR par l’API, obtient une approbation `official=true` liée exactement au SHA de tête API, puis planifie l’AutoMerge par le POST API normal (`MergeWhenChecksSucceed=true`, HTTP 201). Une barrière test-only est placée immédiatement après le `CheckPullMergeable` général dans `services/automerge/automerge.go::handlePullRequestAutoMerge`.

Le test appelle le corps réel de ce worker par un wrapper test-only, dans une goroutine; la livraison depuis la file/queue n’est donc pas exercée. Lorsque le worker s’arrête après son contrôle, une autre requête API authentifiée comme administrateur dismiss l’unique avis et relit `dismissed=true`. Le worker suspendu est relâché. Les GET API relisent alors PR fusionnée et avis toujours dismissed. Le dismiss déclenche aussi un autre contrôle asynchrone, qui observe l’absence d’approbation et refuse; il ne retire pas la décision déjà prise par le worker suspendu.

## Résultats

- Le premier essai a échoué au build : le test utilisait un DTO inexistant `api.MergePullRequestOption`. L’examen des tests amont a identifié `services/forms.MergePullRequestForm`; le test a été corrigé sans changer Forgejo.
- Le second essai compilait mais échouait sur une assertion de test : `models/issues.PullRequest.HeadCommitID` est marqué `xorm:"-"`, donc n’est pas rempli par `GetPullRequestByIndex`. L’assertion redondante a été retirée; la tête exacte reste celle du readback API et le worker la confronte à la ref Git.
- Après correction, le test passe une fois, puis **5/5** avec `-count=5` et **1/1** avec `-race`. Les résultats Go sont `ok forgejo.org/tests/integration` (5,645 s pour cinq exécutions; 9,363 s pour le run race). La commande ciblée, les versions et les journaux complets sont dans ce répertoire.
- `gofmt -d` des deux fichiers test-only AutoMerge et `git diff --check` du scratch n’émettent aucune erreur.

## Conclusion bornée

Dans cette interleaving déterministe, le worker AutoMerge fusionne après révocation de l’approbation, car il ne revalide pas les protections entre le hook après `CheckPullMergeable` et `pull_service.Merge`. La course est distincte des routes API et Web observées en CONT-316 à 318. La preuve utilise le corps de worker réel et le POST de planification HTTP, mais appelle le worker directement; elle ne valide ni le déclenchement normal par queue ni l’ensemble du service.

Cette expérience ne couvre qu’une approbation révoquée, SQLite et cette copie instrumentée. Elle ne couvre pas les changements de règle, statuts de commit, avance concurrente de base, force/manual merge, Actions ou autres writers, PostgreSQL, cluster, serveur amont intact ou production. Aucun gain de productivité n’est mesuré. Ce résultat renforce le besoin d’une précondition vérifiée au point d’écriture pour tout mode de fusion; une lecture et un verrou détenus seulement par le gate ne l’assurent pas. Il ne suffit pas à justifier un fork : le prochain travail doit recenser les autres writers et les possibilités d’un contrôle serveur commun, puis chiffrer compatibilité et coût de rebase/maintenance. Garder Forgejo comme socle non modifié demeure la décision actuelle.
