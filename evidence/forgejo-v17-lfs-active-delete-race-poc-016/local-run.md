# POC-016 — course de suppression avec association active concurrente

Date : 24 octobre 2026. Forgejo officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; test diagnostique uniquement dans `/tmp/forgejo-v17-base-cas-poc`. Go `1.27.3 linux/arm64`, SQLite de test et stockage LFS local temporaire. Aucun code Forgejo copié dans le produit.

## Scénario et résultat

`TestDeleteRepositoryLFSConcurrentActiveAssociationCanLoseObject` bloque le `storage.LFS.Delete` appelé après que la transaction DB de suppression du dépôt A a décidé d’effacer l’OID. Pendant cette fenêtre, le test appelle `NewLFSMetaObject` pour le même OID dans le dépôt B, puis reprend l’effacement. Il constate que la ligne active de B reste lisible alors que le blob est absent.

Le test tagué passe parce qu’il affirme le défaut séquencé; ce n’est pas un test correctif ou un résultat de sûreté. Il montre que le comptage actif/pending POC-012 et la vérification pending POC-014 ne sérialisent pas l’insertion active avec l’effacement physique.

## Commandes et vérification

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify poc_lfs_active_delete_race' ./services/repository \
  -run '^TestDeleteRepositoryLFSConcurrentActiveAssociationCanLoseObject$' -v
```

Résultat : `PASS`, `ok forgejo.org/services/repository 0.255s`; journal complet : [`active-race.log`](active-race.log).

La suite ordinaire sans le tag de diagnostic passe avec `ok forgejo.org/services/repository 1.089s` : [`repository-package.log`](repository-package.log). `gofmt -d` et `git diff --check` sont vides. Empreinte de la source scratch : [`scratch-source-sha256.txt`](scratch-source-sha256.txt).

## Limites et conclusion

Le décorateur impose l’ordre en un processus avec SQLite; le test ne couvre pas une base distante, un stockage S3/MinIO, une route LFS HTTP ou un serveur multi-processus. Il confirme le besoin de synchroniser **toutes** les créations actives et pending, uploads de contenu, GC et suppressions de dépôt sur un état durable commun. Le scratch reste non adopté; aucun fix, worker ni migration n’est ajouté au produit.
