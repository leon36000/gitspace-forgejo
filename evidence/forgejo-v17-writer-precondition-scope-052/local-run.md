# Forgejo v17 — portée réelle de la précondition CAS

Date : 25 octobre 2026. Dépôt scratch : `/tmp/forgejo-v17-base-cas-poc`, Forgejo `v17.0.0`, commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`, Go `go1.27.3 linux/arm64`. Le scratch contenait déjà les modifications CAS/LFS historiques; ce lot n’a modifié aucun fichier de production Forgejo. Il ajoute un seul test d’intégration POC non suivi et ses preuves. Aucun code Forgejo n’est copié dans le produit.

## Résultat observé

La vérification `RequireExpectedBaseCommitID` se trouve dans le hook de pré-réception des updates de branche (`routers/private/hook_pre_receive.go::assertExpectedBaseCommitID`). Ce hook couvre les mises à jour reçues via Git; il n’est pas une barrière commune aux mutations de refs faites directement par des services Forgejo.

Le test scratch `tests/integration/branch_expected_base_policy_poc_test.go::TestPOCExpectedBasePolicyDoesNotGateAPIBranchDelete` utilise le serveur d’intégration local : il crée une branche sans PR, lit son commit, la fait avancer par l’éditeur, puis active `RequireExpectedBaseCommitID`. Le SHA client conservé (`65f1bf27bc3bf70f64657658635e66094edbcb4d`) est périmé face au nouveau SHA (`f4ddb0f094667b379dd2dd73d378c9c13fed8cfc`). Le test envoie ce SHA ancien dans `expected_base_commit_id` à `DELETE /api/v1/repos/user2/repo1/branches/expected-base-delete-poc`. L’API répond `204 No Content` et le test confirme que la branche a disparu.

La route `routers/api/v1/repo/branch.go::DeleteBranch` ne lit que le nom de branche. Elle appelle `services/repository/branch.go::DeleteBranch`, qui récupère le commit courant, supprime ensuite la ref avec `Force: true`, et n’accepte ni ne compare de SHA attendu. La requête de test ne déclenche pas le hook de pré-réception. Ce résultat suffit à réfuter que le booléen testé soit une politique serveur globale pour tous les writers. Il ne constitue pas une expérience de course concurrente pendant l’intervalle lecture-suppression; aucun écrasement concurrent n’est revendiqué.

Commande exacte, exécutée hors ligne dans le scratch :

```sh
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -count=1 -v -timeout=20m -tags sqlite ./tests/integration \
  -run '^TestPOCExpectedBasePolicyDoesNotGateAPIBranchDelete$'
```

Résultat final : code de sortie 0, test 1/1. La première variante utilisait la branche de fixture `branch2`, déjà référencée par une PR; après suppression, un job asynchrone de cette PR a écrit une erreur `GetMergeBase` dans le journal alors que le test lui-même passait. Ce journal brut a été remplacé par la relance finale et n’est pas conservé comme preuve. Le test a été isolé sur `expected-base-delete-poc`, sans PR, puis relancé proprement; seul ce résultat final est compté. `format-and-diff-checks.txt` conserve `gofmt` et `git diff --check`; `scratch-integration-test.log` contient les deux SHA et la réponse HTTP. Les sources et le test sont identifiés par SHA-256 dans `source-and-test-hashes.txt`; elles ne sont pas recopiées ici.

## Couverture et coût d’intégration

L’inventaire ciblé sépare au moins ces chemins : receive-pack Git avec hook; merge/rebase de PR qui transmettent un contexte serveur au hook; API/web de fichiers qui pousse par le chemin temporaire; et suppression API de branche directe. Le POC précédent `forgejo-v17-base-cas-poc-002-lfs-residue` a déjà démontré qu’un refus du writer API de fichier peut laisser une association LFS active, accessible par le dépôt cible public du fixture. Cette preuve n’a pas été rejouée.

Le staging LFS actuellement présent dans le scratch est attaché aux workflows PR de merge/rebase (`services/pull/lfs.go`, `services/pull/merge.go`, `services/pull/update_rebase.go`). Le récupérateur `services/pull/lfs_recovery.go` examine toutes les lignes pending du dépôt dans un chargement non paginé et n’a pas d’appelant de production recensé; le test multiprocessus 051 n’a établi que l’idempotence de promotion sous SQLite fichier. Le writer `services/repository/files::CreateOrUpdateFile` conserve le chemin qui crée une association active avant l’update de ref.

Étendre une obligation serveur à tous les writers demande donc plus que le hook pre-receive : contrats de précondition pour suppression/renommage et écritures directes, comparaison atomique au niveau du ref réellement modifié, compatibilité des clients API/Web qui n’envoient pas aujourd’hui ce champ, protection des chemins AutoMerge/manuellement-merged et publication LFS pending pour les writers génériques. L’audit n’a pas chiffré les heures ni le coût de rebase upstream; il établit que le patch actuel ne satisfait pas la couverture annoncée. Une modification centrale de Forgejo ne doit être envisagée qu’après inventaire de toutes ces routes et preuve que l’adaptateur/les permissions ne suffisent pas au modèle retenu.

## Vérification du Cargo produit

Sur le worktree produit au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, `scripts/test_cargo_license_state.py` passe 7/7 et `scripts/verify_cargo_license_state.py` confirme `license=null`, `license_file=null`, `publish=[]`, décision `unselected` et droits de redistribution `not_assessed`. Aucune licence n’a été sélectionnée.

## Limites et suite

C’est un POC d’intégration Forgejo dans le scratch, pas une modification intégrée au produit, une instance de production, une preuve de toutes les routes ou une comparaison de coût. Il n’utilise ni modèle/LLM ni données privées. Le prochain test utile est de rendre le chemin générique `ChangeRepoFiles` LFS pending jusqu’à la promotion après ref update, puis d’exercer l’interruption avant/après ce point avec récupération idempotente et visibilité LFS; la pagination, le budget par passage, la rétention et l’absence actuelle de worker devront rester explicites. En parallèle, il faut poursuivre l’inventaire des writers directs avant de qualifier un CAS serveur de global.
