# POC-023 — stream LFS lié à un mapping SQLite générationnel

Date : 24 octobre 2026. Forgejo officiel v17.0.0, scratch `/tmp/forgejo-v17-base-cas-poc`, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571`. Le worktree produit n’a reçu aucun code Forgejo; il conserve seulement le test scratch, ses journaux et les empreintes. Go `1.27.3 linux/arm64`.

## Hypothèse et scénario

POC-022 extrayait le stream HTTP de `services/lfs/server.go::DownloadHandler`, mais son `Resolve/Open` test-only utilisait une génération en mémoire. Ce lot ajoute `services/lfs/versioned_download_sqlite_poc_test.go`, protégé par le build tag `forgejo_poc_sqlite_generation_download`. Le test construit un registre SQLite local avec une ligne de mapping courante et des rangées de génération, ainsi qu’un vrai `storage.LocalStorage` sous `t.TempDir()`.

Le test écrit et promeut une génération initiale, puis appelle le vrai helper `streamVersionedContent` avec `Range: bytes=3-6`. Il réserve ensuite une seconde génération persistée et simule la promotion par un autre writer entre `Resolve` et `Open`. `Open` relit la ligne SQLite et refuse le jeton de génération devenu obsolète. Un readback suivant utilise la nouvelle génération courante.

## Défaut détecté puis correction du test

Le premier run a échoué dans le POC seulement : la requête XORM `operation_id` ne correspondait pas au nom SQL produit par le mapper par défaut pour `OperationID`. Le test a été configuré avec `names.GonicMapper{}` avant la synchronisation des tables. Le test ciblé et la suite de paquet passent ensuite. Le journal initial est conservé dans `go-test-first.log`; ce défaut n’était pas dans le serveur Forgejo.

## Vérifications

Depuis `/tmp/forgejo-v17-base-cas-poc` :

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_download forgejo_poc_generation_streamed_download' ./services/lfs -run '^(TestPOCSQLiteMappingBindsStreamToResolvedGeneration|TestPOCStreamedDownload)' -count=1 -v
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_download forgejo_poc_generation_streamed_download' ./services/lfs -count=1
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/gofmt -d services/lfs/versioned_download_sqlite_poc_test.go
git diff --check -- services/lfs/versioned_download_sqlite_poc_test.go
```

Les trois tests ciblés passent : les deux précédents POC-022 et le nouveau cas SQLite, `ok forgejo.org/services/lfs 0.163s`. Le paquet complet passe également (`0.177s`). Gofmt n’émet aucune différence; `git diff --check` passe. Les logs, copie exacte du test, versions et hashes du test et du `server.go` scratch sont dans ce dossier.

## Limites et suite

Le nouveau code est un test d’architecture seulement; il n’ajoute ni migration, ni writer, ni `ContentStore` réel, ni wiring de `DownloadHandler` vers SQLite. La promotion intercalée pendant `Open` est une simulation séquentielle déterministe, pas une course multi-processus. SQLite/LocalStorage ne prouvent pas PostgreSQL, MySQL, S3, MinIO, crash transactionnel, rétention/GC, suppression concurrente, uploads LFS ou client Git LFS. Les endpoints Forgejo et l’intégration webhook ne sont pas exercés par ce POC.

Prochaine étape : déterminer un seam d’injection limité au routeur/test, traverser `Resolve/Open` SQLite depuis une requête HTTP authentifiée, puis examiner l’ensemble des readers, writers, suppressions et GC avant d’envisager la moindre connexion de production. Aucun code Forgejo n’est adopté ou copié au produit; le coût de maintenance du fork n’est toujours pas chiffré.
