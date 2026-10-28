# POC-018 — support SDK d’un delete LFS par génération (24 octobre 2026)

## Question

Peut-on exprimer un effacement MinIO ciblant précisément une génération d’objet, au lieu du chemin `Delete(path)` actuel qui peut viser une republication du même chemin ?

## Source examinée

- Forgejo officiel v17.0.0, HEAD scratch `0d791421f85ca13a63b665053d81ea765fbbc571`, copie de travail `/tmp/forgejo-v17-base-cas-poc`; cette copie contient des POC expérimentaux non adoptés.
- `go.mod:79` épingle `github.com/minio/minio-go/v7 v7.1.0`.
- Dans cette version du SDK, `api-remove.go::RemoveObjectOptions` possède `VersionID`; `Client.removeObject` encode cette valeur comme paramètre `versionId` de la requête DELETE.
- Le type SDK `UploadInfo` (`api-datatypes.go:131–160`) contient `VersionID`. Dans Forgejo, `modules/storage/minio.go::Save` reçoit `uploadInfo` mais retourne seulement `uploadInfo.Size`; `Delete` appelle `RemoveObject` avec des options vides. `NewMinioStorage` interroge `GetBucketVersioning`, mais le commentaire aux lignes 119–120 précise que cet appel vérifie seulement la validité générale des paramètres, pas l’activation du versionnage.
- `modules/storage/storage.go::ObjectStorage` ne retourne pas d’identifiant de génération au `Save` et `Delete(path)` n’accepte ni version ni précondition. LocalStorage supprime le chemin avec `util.Remove`.

## Test exécuté

Un nouveau test Go tagué `forgejo_poc_lfs_generation` crée un serveur HTTP en mémoire, appelle le SDK MinIO `RemoveObject` avec `VersionID="generation-old"`, puis vérifie méthode, chemin et paramètre `versionId` reçu. Ce test vérifie la construction de la requête SDK uniquement.

Commande depuis `/tmp/forgejo-v17-base-cas-poc` :

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_generation' ./modules/storage -run '^TestPOCVersionSpecificDeleteCarriesVersionID$' -count=1 -v
```

Résultat : `PASS`, paquet `forgejo.org/modules/storage`, 0.009 s. Go `1.27.3 linux/arm64`; `gofmt -d` est vide. Le test et son résultat sont inclus à côté de ce rapport; SHA-256 du test scratch : `b127e5db91dee23624adab7e90ac225c15f8a22b9986e1375f625afcbab57fae`.

## Interprétation et limites

Le SDK permet un DELETE ciblé par version, mais l’adaptateur Forgejo actuel jette l’identifiant retourné à l’upload et ne passe pas de version au delete. Le bucket peut être non versionné ou compatible S3 sans support équivalent; le probe de construction HTTP ne démontre ni l’activation de versioning, ni les sémantiques MinIO/S3, ni une réponse tardive d’un backend réel. Cette voie n’est donc pas une correction prête à l’emploi et ne couvre pas LocalStorage.

Une option uniforme à évaluer est un chemin d’objet immuable par génération, avec une référence OID → génération persistée; une suppression retardée ne peut alors viser le chemin d’une génération ultérieure. Cela déplacerait une part importante du travail vers le schéma, la migration des objets existants, les lecteurs LFS, le protocole d’upload, les stages, les GC et la restauration. À défaut de génération immuable, la décision après résultat de stockage ambigu doit rester fail-closed : garder la réservation et empêcher une republication sur le même chemin jusqu’à résolution.

Aucun code Forgejo n’a été copié dans le produit, aucun backend distant n’a été contacté, et aucune correction de production n’est démontrée.

## Portée statique initiale du mapping LFS

Dans le même HEAD v17, `rg` limité aux fichiers `*.go` de production, hors `_test.go`, trouve 11 constructions de `lfs.NewContentStore`/`lfs_module.NewContentStore` réparties dans huit fichiers : `modules/repository/repo.go`, `services/repository/lfs.go`, `services/repository/files/{upload,update}.go`, `routers/web/repo/setting/lfs.go`, `services/mirror/mirror_push.go`, `services/pull/lfs.go` et quatre emplacements dans `services/lfs/server.go`. Cinq usages directs de `storage.LFS` se trouvent dans `routers/web/repo/setting/lfs.go` (delete), `routers/web/repo/download.go` (URL), `routers/api/v1/repo/file.go` (URL), `services/lfs/server.go` (URL) et `cmd/dump.go` (iteration/exports). Dix dérivations `Pointer.RelativePath()` ou `LFSMetaObject.RelativePath()` sont dans `cmd/migrate_storage.go`, `routers/web/repo/download.go`, quatre emplacements dans `modules/lfs/content_store.go` (Get/Put/Exists/Verify), `services/repository/delete.go`, `services/repository/lfs.go`, `routers/api/v1/repo/file.go` et `services/lfs/server.go`. `modules/lfs/content_store.go` centralise les chemins de Get/Put/Exists/Verify; les autres chemins servent aux URL, suppression, GC, migration et dump. Ces recherches sont reproductibles et balisent les composants à adapter.

Ce comptage indique une migration inter-composants, mais ne donne pas un effort en jours et n’est pas une preuve exhaustive : méthodes indirectes, SQL/chemins littéraux, scripts, plugins et readers tiers restent à recenser.
