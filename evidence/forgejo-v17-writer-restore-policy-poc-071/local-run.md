# Forgejo v17 — restauration web d’une branche sous précondition expérimentale (WRITER-RESTORE-POLICY-071)

Date : 25 octobre 2026. Forgejo officiel v17.0.0, source de base `0d791421f85ca13a63b665053d81ea765fbbc571`. L’identifiant complet `WRITER-RESTORE-POLICY-071` distingue ce cas du POC de promotion LFS également numéroté 071. Le test a été exécuté dans `/tmp/forgejo-v17-base-cas-poc`, un scratch préexistant et fortement modifié. Ce lot y ajoute seulement le test `branch_expected_base_restore_policy_poc_test.go`; il ne modifie pas la logique Forgejo et ne copie aucun code amont dans le produit.

## Résultat

Le test crée une branche, la supprime par l’API puis active le hook expérimental `RequireExpectedBaseCommitID`. Il appelle ensuite la restauration web avec le tombstone de cette branche. Le `pre-receive` refuse le push local avec HTTP 409 : il reçoit une création de ref à OID nul, mais aucun contexte de pull request ni `ExpectedBaseCommitID` serveur. La requête web termine sa redirection HTTP 200; la ref n’est pas recréée et le tombstone reste marqué supprimé.

Le log détaille la cause dans `log.Error`, mais `RestoreBranchPost` renvoie seulement le flash générique `restore_failed`. Le test logger signale cet appel attendu à `log.Error` comme `FATAL ERROR` alors que `go test` passe. Cet artefact du banc est conservé et limite l’interprétation du journal.

## Vérification

Commande exacte et versions : [commands.txt](commands.txt), [tool-versions.txt](tool-versions.txt). Le test ciblé passe 1/1, code retour 0 (`PASS`, `ok forgejo.org/tests/integration 1.200s`). `gofmt -d` est vide; contrôle de blancs et hashes : [source-sha256.txt](source-sha256.txt). L’extrait conservé est explicitement partiel : [test-output-excerpt.txt](test-output-excerpt.txt).

Sources : au commit officiel, `routers/web/repo/branch.go::RestoreBranchPost` lit le tombstone, lance `git.Push` et masque l’échec avec `repo.branch.restore_failed`; `services/repository/branch.go::DeleteBranch` conserve le tombstone; `models/git/branch.go::GetDeletedBranchByID` le lie au dépôt et à son état supprimé. Le hook `routers/private/hook_pre_receive.go::assertExpectedBaseCommitID` et son flag sont des modifications expérimentales du scratch, pas du code du commit officiel.

Nouveau test : [`branch_expected_base_restore_policy_poc_test.go`](/tmp/forgejo-v17-base-cas-poc/tests/integration/branch_expected_base_restore_policy_poc_test.go), SHA-256 indiqué dans le fichier de hashes. Le serveur de test est initialisé en processus avec dépôt Git temporaire et SQLite; ce n’est pas un binaire Forgejo autonome.

## Ce que cela change pour la décision

La politique expérimentale, si elle est appliquée au chemin de réception, bloque aussi cette voie de restauration. Pour conserver la fonctionnalité, il faudra décider si la création sur ref absente se protège par l’OID nul, transporter une précondition depuis une source de confiance et rendre l’erreur actionnable dans l’UI. Le test ne justifie pas une exemption implicite ni un contournement du hook.

Cette tranche ajoute un cas à la matrice de compatibilité, mais ne démontre toujours pas une précondition atomique sur tous les writers. Elle n’est ni un correctif produit ni une adoption d’un fork. Elle ne chiffre pas le coût upstream en heures; le scratch agrège d’autres changements non attribuables. Aucun LLM, service externe, secret, coût, production, PR ou fusion n’a été utilisé.
