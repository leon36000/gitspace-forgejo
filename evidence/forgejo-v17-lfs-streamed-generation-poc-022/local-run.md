# POC-022 — lecture LFS streamée liée à une génération (24 octobre 2026)

## Source et portée

Expérience dans `/tmp/forgejo-v17-base-cas-poc`, clone scratch de Forgejo officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le changement scratch extrait le stream HTTP de `services/lfs/server.go::DownloadHandler` dans un helper utilisé par ce handler. Une interface locale porte `context.Context` et une référence `{oid, generation}`; l'adaptateur par défaut ne sait encore lire que l'ancien chemin (génération `0`). Le test utilise un faux registre de générations et le vrai `storage.LocalStorage`; aucun code source Forgejo n'est copié ici. Les empreintes de la base, du fichier scratch modifié et des deux tests sont dans `source-hashes.txt`.

Le handler conserve l'authentification et la recherche `LFSMetaObject` avant d'appeler le helper. Un test d'intégration additionnel passe par le routeur Forgejo local, la base de test et l'authentification existante, mais utilise le stockage legacy par défaut. Les tests de générations n'exécutent pas de routeur ou middleware Forgejo et n'ont pas de registre DB.

## Échec trouvé puis correction

Le premier test rouge du helper a reproduit le header `Content-Range` v17 avec `Range: bytes=3-6` sur un objet de 10 octets : Forgejo répondait `bytes 3-6/7`, en utilisant `size-fromByte` comme taille totale. Le test attendait `bytes 3-6/10`. La correction locale dans le scratch utilise la taille complète. Le test du routeur confirme ensuite le même contrat sur la route réelle (`bytes 1-3/10` sur 10 octets) et le corps `234`. Ce n'est ni un patch upstream ni un correctif intégré au produit.

## Commandes et résultats

Depuis `/tmp/forgejo-v17-base-cas-poc` (Go `1.27.3 linux/arm64`) :

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_generation_streamed_download' ./services/lfs -run '^TestPOCStreamedDownload' -count=1 -v
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify' ./services/lfs -count=1
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_generation_streamed_download' ./tests/integration -run '^TestPOCLFSDownloadRangeReportsFullObjectSize$' -count=1 -v
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/gofmt -d services/lfs/server.go services/lfs/versioned_download_poc_test.go tests/integration/lfs_generation_stream_poc_test.go
git diff --check -- services/lfs/server.go services/lfs/versioned_download_poc_test.go tests/integration/lfs_generation_stream_poc_test.go
```

Après correction, les deux tests helper passent (`0.083s`): lecture de la génération courante avec corps/range correct; refus d'un mapping manquant (`404`), d'une clé physique absente (`404`) et d'une génération supplantée entre résolution et ouverture (`409`). La commande sans build tag compile le paquet et indique `[no test files]`. Le test d'intégration HTTP réel passe (`ok forgejo.org/tests/integration 0.610s`) avec SQLite, route, authentification, association LFS et LocalStorage. Pendant son teardown, le logger du harnais émet des erreurs `context canceled`, mais l'assertion HTTP (`206`, header et corps) réussit et le processus sort avec code 0. Journaux rouge/verts, integration, gofmt et diff check sont conservés ici.

## Ce que cela n'établit pas

Le registre générationnel des tests est simulé en mémoire et ne prouve aucune course transactionnelle. L'adaptateur réellement appelé par le handler reste legacy (`ContentStore.Get`); aucune table de mapping, migration, upload générationnel, URL signée, LFS client complet, stockage distant ou reprise après crash n'a été testée. Le run HTTP ne valide donc que le chemin legacy et le framing du stream, pas l'activation d'une génération persistée. Aucune contribution upstream ou redistribution n'est prévue, conformément à `CONTRIBUTING.md:5–7`.
