# FORGEJO-V17-MERGE-WHITELIST-RENAME-POC-076

Date locale du test : 25 octobre 2026. Ce POC reste dans `/tmp/forgejo-v17-base-cas-poc`, basé au tag officiel Forgejo `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Ce scratch est déjà fortement modifié par des expériences CAS/LFS antérieures; aucun de ses changements de production n’est copié dans le worktree produit. Le seul fichier ajouté pour ce POC est `tests/integration/merge_whitelist_branch_rename_bypass_poc_test.go`. Les sources de route/permission vérifiées sont inchangées par rapport à ce commit: voir `source-hashes.txt`.

## Parcours exercé

Le harnais SQLite crée un compte de test `forge-merger`, non-administrateur, lui donne le rôle collaborateur `write`, et le place seul dans la merge whitelist d’une règle exacte sur la branche par défaut. La règle interdit le push et s’applique aux admins. L’API Branch indique alors `protected=true`, `user_can_merge=true`, `user_can_push=false`. Ce compte crée une branche PR et la fusionne par la route API normale alors que `RequireExpectedBaseCommitID=true`; le readback API confirme `merged=true` et la branche avance.

Avec le même compte de test et le même cookie/token :

- API `DELETE /branches/{base}` : HTTP 403.
- Web `POST /settings/rename_branch` : HTTP 404 (route protégée par `reqRepoAdmin`).
- Git HTTP direct vers la branche protégée : refus du hook; pour isoler cette politique, le test désactive uniquement `RequireExpectedBaseCommitID` pendant cette sonde, puis le restaure. Le push reçoit une erreur mentionnant la branche protégée.
- API `PATCH /branches/{base}` avec `RequireExpectedBaseCommitID=true` : HTTP 204. Le SHA de la ref est conservé sous le nouveau nom; l’ancien nom disparaît; la règle de protection suit la branche renommée. Un GET confirme que la branche renommée reste protégée, fusionnable pour cet acteur, et non pushable.

La différence montre qu’un merge-whitelist et l’interdiction de push ne constituent pas une autorité merge-only : l’API de renommage réutilise `reqRepoWriter`, puis appelle `RenameBranch` directement. Dans ce test, le compte autorisé à merger peut aussi renommer la branche protégée et déplacer le nom du défaut. Cela contredit l’hypothèse qu’une configuration de rôle Forgejo seule exclut tous les writers hors route de merge.

## Résultat reproductible

Commande : `GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags sqlite ./tests/integration -run '^TestPOCMergeWhitelistActorCanRenameProtectedBranch$' -count=1`. Résultat : `ok forgejo.org/tests/integration` (un test ciblé). `gofmt -d` ne produit aucune différence. Sortie conservée dans `test.log`; toolchain dans `toolchain.txt`.

## Sources et limites

- Le test ajouté et ses assertions : `tests/integration/merge_whitelist_branch_rename_bypass_poc_test.go:29-138` dans le scratch.
- Le registre API utilise `reqRepoWriter` pour DELETE/PATCH de branche : `routers/api/v1/api.go:895-901`; le renommage appelle `repo_service.RenameBranch` : `routers/api/v1/repo/branch.go:393-448`.
- L’implémentation de `RenameBranch` modifie ref et métadonnées/branche par le service, et `models/git/branch.go:309-392` déplace la règle de protection avec le nom. Hashes des fichiers de route/service/modèle identiques au blob de l’upstream HEAD, établis dans `source-hashes.txt`.
- L’API Web des paramètres est groupée sous `reqRepoAdmin` : `routers/web/web.go:1348` (scratch); le handler de suppression utilise le contrôle de branche protégée : `services/repository/branch.go:435-452`.
- Le run utilise un compte local recréé au même login et avec un rôle équivalent; il n’utilise ni le credential ni la configuration complète de l’instance MERGE-020. L’identité/token exacts de MERGE-020 n’étaient pas présents dans les artefacts sûrs et aucun secret/config runtime n’a été lu. La fixture n’active pas l’approbation reviewer, `block_on_outdated_branch` ni toutes les autres règles de MERGE-020.
- Bien que les sources de route concernées correspondent à l’upstream HEAD, le processus entier tourne dans le scratch CAS/LFS modifié, pas dans un checkout propre isolé. SQLite seulement, un seul processus, pas de cluster/PostgreSQL. Ce test couvre API, Web (refus) et Git HTTP (refus) pour cette route et ce rôle; il ne couvre pas SSH, autres helpers internes, race avec writers concurrents, ni la matrice exhaustive des refs.
- Pas de correction du produit, fork adopté, publication ou modification d’instance de production. La conclusion limitée est un problème d’autorisation path-specific dans Forgejo v17 pour cette combinaison de permissions; remède possible à évaluer : broker/point d’accès permettant une liste positive des routes pour le credential de merge, avec isolation réseau et contrôle des chemins Git, ou politique/endpoint serveur upstream. Un proxy seul n’est pas déclaré suffisant tant que tous les accès directs ne sont pas exclus.
