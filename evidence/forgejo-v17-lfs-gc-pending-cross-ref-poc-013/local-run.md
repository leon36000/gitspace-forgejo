# POC-013 — GC LFS avec référence pending dans un autre dépôt

Date : 24 octobre 2026. Source : Forgejo officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, scratch `/tmp/forgejo-v17-base-cas-poc`. Environnement : Go `1.27.3 linux/arm64`, Git `2.43.0`, SQLite de test et stockage LFS temporaire. Ce POC ajoute un test seulement; aucun source Forgejo n’est copié au produit.

## Contrat vérifié

Le test `TestGarbageCollectLFSMetaObjectsPreservesPendingReferenceInAnotherRepository` utilise le dépôt de fixture `user2/lfs` comme dépôt A et crée un dépôt B avec uniquement un stage pending. Il stocke un objet synthétique dans le content-store et associe initialement sa métadonnée active à A, sans pointeur dans l’arbre Git de la fixture. Le GC réel `GarbageCollectLFSMetaObjectsForRepo` classe donc l’association active comme orpheline et la retire. Le test vérifie ensuite que :

- la métadonnée active a disparu de A;
- aucune association active n’est visible au dépôt B via `GetLFSMetaObjectByOid`;
- le stage pending de B existe encore;
- le blob du content-store reste présent.

## Résultats

Test ciblé sous SQLite : `PASS`, `ok forgejo.org/services/repository 0.283s`; sortie [`focused-green.log`](focused-green.log). Le GC journalise `Collected 2/2 orphaned/5 total LFSMetaObjects in user2/lfs. 1 removed from storage.` La suppression d’un seul objet au lieu de deux correspond au blob protégé par le stage B.

Vérification de sensibilité : le helper `CountPendingLFSMetaObjectsByOid` a été temporairement muté pour renvoyer zéro. Le même test échoue sur l’existence du blob, et le GC journalise alors `2 removed from storage`. Le helper a été restauré et le test ciblé repasse; trace du rouge de mutation : [`mutation-red.log`](mutation-red.log). Ce n’est pas un run de Forgejo officiel non modifié, puisque le modèle de stages appartient au prototype CAS.

Après restauration, la suite entière du paquet `./services/repository` passe sans cache : `ok forgejo.org/services/repository 1.016s`, journal [`repository-package.log`](repository-package.log). `gofmt -d` est vide et `git diff --check` passe; journaux [`gofmt-diff.log`](gofmt-diff.log) et [`diff-check.log`](diff-check.log). SHA-256 du test scratch : [`scratch-source-sha256.txt`](scratch-source-sha256.txt).

Commande ciblée :

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify' ./services/repository \
  -run '^TestGarbageCollectLFSMetaObjectsPreservesPendingReferenceInAnotherRepository$' -v
```

## Limites

La vérification traverse la fonction de GC pour un dépôt et le stockage local temporaire. Elle ne teste pas le GC global, un serveur/API LFS, un dépôt privé, le stockage objet distant, une migration, ni la concurrence entre GC et création d’un nouveau stage. Elle démontre uniquement que le compteur pending expérimental existant protège le blob dans le scénario séquentiel testé. Cela ne prouve ni la sûreté du protocole complet ni un motif d’adoption du fork.
