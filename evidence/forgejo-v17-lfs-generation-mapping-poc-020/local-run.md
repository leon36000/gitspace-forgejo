# POC-020 — registre DB par génération et reprise de publication LFS (24 octobre 2026)

## Hypothèse et portée

Des clés physiques immuables par génération permettent de séparer une ancienne suppression d'une publication plus récente, à condition qu'une ligne DB soit l'unique mapping OID→clé lue. Une réservation persistée avant l'upload doit permettre de reprendre après un arrêt avant promotion; la promotion et la mise à jour du mapping doivent être atomiques. Une reprise doit contrôler le contenu stocké, être idempotente, et ne pas laisser un ancien travail de récupération faire reculer une génération plus récente.

POC scratch sur Forgejo officiel v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. `models/git/lfs_generation_mapping_poc_test.go` est un test scratch local, sous le build tag `forgejo_poc_lfs_generation_mapping`; il définit ses propres tables d'essai et n'ajoute ni migration ni chemin de production. Le test utilise la DB de tests de `models/git` (SQLite avec les tags ci-dessous) et un vrai `storage.LocalStorage` sous `t.TempDir()`.

## Protocole joué

1. Une transaction crée un enregistrement `pending` avec OID, digest/taille attendus, génération, identifiant d'opération idempotent et clé physique unique `generation/<oid>/<n>/<operation>`.
2. Les octets sont sauvegardés sur cette clé. Avant promotion, le mapping courant conserve l'ancienne génération.
3. La fonction de reprise, sans état en mémoire, rouvre le stockage et vérifie taille, SHA-256 et égalité OID/digest. Une transaction marque l'ancienne génération `retired`, la nouvelle `active` et remplace le mapping courant. La répétition ne change pas le résultat.
4. L'essai simule un arrêt après sauvegarde des octets et avant promotion, puis rejoue la fonction à partir des seules lignes DB et du stockage. Une suppression de l'ancienne clé après activation laisse les octets actifs intacts.
5. L'objet absent et un contenu corrompu de même taille ne sont pas promus. Une récupération de génération ancienne après la promotion d'une plus récente n'annule pas le mapping courant.

## Commandes et résultat

Depuis `/tmp/forgejo-v17-base-cas-poc` :

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go version
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/gofmt -d models/git/lfs_generation_mapping_poc_test.go
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_generation_mapping' ./models/git -run '^TestPOCRecoverableGenerationMappingAndDelayedDelete$' -count=1 -v
```

Résultat du test ciblé : `PASS`, `ok forgejo.org/models/git 0.197s`; outil `go1.27.3 linux/arm64`; la sortie gofmt est vide. Les sorties sont conservées dans ce dossier. La source scratch est épinglée par SHA-256 dans `scratch-source-sha256.txt`.

## Conclusion et limites

Le test valide un protocole candidat minimal sur une base SQLite de tests et LocalStorage : réservation persistée → octets immuables → validation à la reprise → bascule atomique du mapping; l'absence, l'altération ou le retard d'une génération ne deviennent pas un mapping actif. C'est une avancée au-delà de POC-019, qui ne testait que deux chemins physiques.

Ce n'est **pas** une implémentation Forgejo : les endpoints, `ContentStore`, URL/presigned URL, upload LFS, association `LFSMetaObject`, stages PR, import/mirror, migration des clés legacy, GC, suppression de dépôt et toutes les autres voies d'écriture ne lisent ni n'écrivent ces tables. Le test ne tue pas un processus : il simule la reprise en rappelant une fonction sans état mémoire, dans le même processus et la même transaction de test. Il ne couvre pas deux promoteurs concurrents, PostgreSQL/MySQL, MinIO/S3, crash pendant la transaction, échéance/lease, inventaire et rétention des stages, lectures déjà ouvertes, stockage objet réellement versionné ni les chemins hors modèles. L'opération « plus grande génération gagnante » est une règle candidate; sa cohérence multi-backend reste à prouver. Aucun coût upstream n'est chiffré; aucun code Forgejo n'est adopté ou copié au produit et le fork reste expérimental.
