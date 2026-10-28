# POC-014 — course pending après commit de suppression LFS

Date : 24 octobre 2026. Source : Forgejo officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, scratch `/tmp/forgejo-v17-base-cas-poc`. Environnement : Go `1.27.3 linux/arm64`, SQLite de test, stockage LFS local temporaire. Aucun code source Forgejo n’est copié au produit.

## Propriété et méthode

POC-012 protège un dépôt B lorsque son stage existe au moment où la suppression de A compte les références. Il restait à vérifier la fenêtre qui suit le commit DB et précède l’effacement du blob. Le test diagnostique `TestDeleteRepositoryLFSConcurrentStageCanLoseObject` remplace `storage.LFS` par un décorateur bloquant au moment de `Delete` :

1. il lance la suppression de A sans référence externe;
2. l’appel au stockage confirme que la transaction DB a déjà décidé l’effacement;
3. pendant que le fichier existe encore, il crée et commit le stage de B;
4. il reprend l’effacement et relit l’état DB et du content-store.

Le test passe parce qu’il affirme explicitement le résultat vulnérable observé : le stage B est présent, mais le blob a disparu. C’est un diagnostic de défaut, pas un test de sûreté réussi. Il est derrière le build tag `poc_lfs_delete_race` et n’est pas exécuté par la suite Go ordinaire.

## Vérifications

Diagnostic ciblé :

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify poc_lfs_delete_race' ./services/repository \
  -run '^TestDeleteRepositoryLFSConcurrentStageCanLoseObject$' -v
```

La trace [`race-diagnostic.log`](race-diagnostic.log) montre `PASS`, `ok forgejo.org/services/repository 0.267s`; ce PASS signifie que le défaut séquencé est reproduit.

La suite ordinaire, sans le build tag diagnostic, passe : `ok forgejo.org/services/repository 1.067s`; voir [`default-package.log`](default-package.log). `gofmt -d` est vide et `git diff --check` passe. Le test scratch est empreinté dans [`scratch-source-sha256.txt`](scratch-source-sha256.txt).

## Conclusion et limite

La fenêtre transaction DB → stockage est une course de données reproductible, même après le comptage des références pending dans POC-012. Une simple relecture avant l’effacement ne donnerait pas d’atomicité. Avant adoption, il faut un protocole qui sérialise la création de références et la suppression physique, survit au crash entre ces opérations, traite les writers actifs/pending et laisse une récupération idempotente. Un verrou local en mémoire ne suffirait pas à garantir la sûreté multi-processus.

Le décorateur impose l’ordre dans un seul processus sous SQLite; le test ne valide pas une correction, une base SQL de production, un backend objet, un serveur LFS, ni une exécution distribuée. Aucun correctif n’est proposé dans ce POC et le fork reste non adopté.
