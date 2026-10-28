# POC-015 — GC global et référence LFS pending seule

Date : 24 octobre 2026. Forgejo officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; modifications de test non adoptées dans `/tmp/forgejo-v17-base-cas-poc`. Go `1.27.3 linux/arm64`, base de test SQLite, stockage objet local temporaire. Aucun code Forgejo copié au produit.

## Hypothèse vérifiée

POC-013 appelait directement `GarbageCollectLFSMetaObjectsForRepo`. Ici, le test `TestGlobalGarbageCollectLFSMetaObjectsPreservesPendingOnlyReference` passe par le point d’entrée global `GarbageCollectLFSMetaObjects`, qui énumère les dépôts à partir des associations LFS actives. Le test crée un OID actif orphelin dans A et une unique référence pending dans B, lance le GC global, puis vérifie : l’association active A a disparu, B ne devient pas visible comme association active, la ligne pending de B reste présente et le contenu du blob existe toujours.

Le log montre que le GC global visite `user2/lfs`, collecte ses deux métadonnées orphelines du fixture et retire un objet physique; l’assertion vérifie spécifiquement la conservation de l’OID partagé avec le stage B. Le test ne prétend pas que le GC traite les stages sans métadonnée active comme work items : ceux-ci ne sont pas énumérés par ce chemin global. Un protocole de rétention/nettoyage pending reste à concevoir.

## Vérifications reproductibles

Test ciblé :

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify' ./services/repository \
  -run '^TestGlobalGarbageCollectLFSMetaObjectsPreservesPendingOnlyReference$' -v
```

Résultat : `PASS`, `ok forgejo.org/services/repository 0.249s`; trace : [`global-gc-test.log`](global-gc-test.log).

Suite du paquet :

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify' ./services/repository
```

Résultat : `ok forgejo.org/services/repository 1.209s`; trace : [`repository-package.log`](repository-package.log). `gofmt -d services/repository/lfs_test.go` et `git diff --check` ne produisent aucune sortie. Empreinte du test scratch : [`scratch-source-sha256.txt`](scratch-source-sha256.txt).

## Portée

Cela étend POC-013 au point d’entrée GC global, toujours séquentiellement, avec SQLite et stockage local. Le test ne vérifie ni concurrence, ni crash, ni stockage S3/MinIO, ni migration, ni récupération/expiration des stages, ni agent réel. Aucun code Forgejo n’est copié dans le produit; le fork reste non adopté.
