# POC-012 — suppression de dépôt et références LFS pending inter-dépôts

Date : 24 octobre 2026. Source : Forgejo officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, scratch `/tmp/forgejo-v17-base-cas-poc`. Environnement : Go `1.27.3 linux/arm64`, Git `2.43.0`, SQLite de test, stockage LFS local temporaire. Aucun code source Forgejo n’est copié dans le dépôt produit; ce patch n’est ni adopté ni déployé.

## Question et comportement attendu

Le dépôt A porte la dernière association LFS active d’un OID. Le dépôt B ne porte que le stage pending du même OID. Supprimer A ne doit pas effacer le contenu requis par B. La suppression de chaque dépôt doit retirer ses propres stages. Le blob ne doit être supprimé qu’après disparition des références actives et pending externes.

## Test rouge sur la suppression officielle

Le test `TestDeleteRepositoryPreservesPendingLFSReferencesAcrossRepositories` a été exécuté contre la version `HEAD:services/repository/delete.go`, avec le modèle pending et le test du POC CAS déjà présents. Il échoue sur trois assertions : le blob n’existe plus après suppression de A; le stage de A reste; le stage de B reste après suppression de B. Sortie : [`baseline-red.log`](baseline-red.log). Un premier lancement avant activation de `setting.LFS.StartServer` a échoué au setup (`LFS isn't enabled`) et n’est pas utilisé comme preuve fonctionnelle.

Pour produire ce rouge, le fichier de suppression expérimental a été sauvegardé, remplacé temporairement par `git show HEAD:services/repository/delete.go`, puis restauré avant le test vert final. Aucun autre fichier scratch n’a été restauré.

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify' ./services/repository \
  -run '^TestDeleteRepositoryPreservesPendingLFSReferencesAcrossRepositories$' -v
```

## Changement expérimental

Dans `services/repository/delete.go`, la transaction de suppression charge les associations actives et pending du dépôt supprimé, déduplique les OID, compte les références dans les autres dépôts, et ne planifie l’effacement du contenu que lorsqu’il n’existe aucune référence externe. Elle retire ensuite les lignes actives et pending du dépôt courant. Le test crée réellement l’objet dans le stockage LFS temporaire, vérifie sa survie après suppression de A alors que le stage de B subsiste, puis vérifie le nettoyage du stage de B et du contenu après suppression de B.

## Vérifications

Commande ciblée :

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify' ./services/repository \
  -run '^TestDeleteRepositoryPreservesPendingLFSReferencesAcrossRepositories$' -v
```

Résultat final : `PASS`, `ok forgejo.org/services/repository 0.258s`; journal [`final-focused-green.log`](final-focused-green.log). Le run initial ciblé après correction a également passé (`0.248s`, [`focused-green.log`](focused-green.log)). Suite du paquet touché :

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify' ./services/repository
```

Résultat : `ok forgejo.org/services/repository 0.943s`, journal [`repository-package.log`](repository-package.log). `gofmt -d` est vide et `git diff --check` réussit; journaux [`gofmt-diff.log`](gofmt-diff.log) et [`diff-check.log`](diff-check.log). Les deux fichiers scratch touchés par ce POC sont empreintés dans [`scratch-source-sha256.txt`](scratch-source-sha256.txt).

Le helper `forgery.CreateRepository` émet un message de journal `CloseRepoBranchesPulls failed: no such file or directory` pendant la suppression, mais l’opération retourne sans erreur et les assertions DB et stockage passent. Ce test exerce la transaction de suppression et le stockage temporaire, pas un serveur Forgejo vivant.

## Limites et prochaine vérification

Le test est séquentiel. La transaction DB se termine avant que le fichier soit supprimé du stockage, comme dans le chemin existant; une nouvelle association pending concurrente peut théoriquement apparaître dans cette fenêtre. Le POC n’établit pas l’absence de cette course et ne couvre pas le GC séparé, la migration de schéma, un backend objet distant, le protocole git-lfs, les interruptions après commit DB ou une instance multi-processus. Il faut traiter verrouillage/réconciliation et suppression physique comme un protocole distinct avant toute adoption.

**Décision :** garder la correction seulement dans le scratch Forgejo v17. Elle ferme le défaut séquentiel démontré, mais ne suffit pas à justifier le fork ni une garantie de sûreté concurrente. Forgejo reste le socle candidat via interfaces; aucun déploiement n’est demandé ou effectué.
