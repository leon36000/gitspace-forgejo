# POC-021 — traversée du vrai `ContentStore` par un adapter de mapping (24 octobre 2026)

## Question

POC-020 prouvait un mapping et une reprise avec des tables d'essai mais n'appelait pas `modules/lfs/content_store.go`. Ce POC vérifie si l'interface `ObjectStorage` peut router les méthodes réelles `ContentStore.Get`, `Put`, `Exists` et `Verify` vers des clés par génération, avec un fallback de lecture des chemins legacy.

Scratch Forgejo officiel v17.0.0 à `0d791421f85ca13a63b665053d81ea765fbbc571`. La seule source scratch ajoutée est un test local sous build tag `forgejo_poc_lfs_content_store_generation`; son SHA-256 est dans `scratch-source-sha256.txt`. Le code de production Forgejo n'est pas modifié par ce POC. L'adapter et les modèles sont définis dans le test seulement; il n'existe aucune migration ou wiring à `NewContentStore`.

## Commande et résultat

Depuis `/tmp/forgejo-v17-base-cas-poc` :

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go version
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/gofmt -d modules/lfs/content_store_generation_poc_test.go
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_content_store_generation' ./modules/lfs -run '^TestPOCContentStoreUsesGenerationMappingAndLegacyFallback$' -count=1 -v
```

Go `1.27.3 linux/arm64`; gofmt ne produit aucune différence; test ciblé `PASS`, `ok forgejo.org/modules/lfs 0.079s`. Le test instancie le vrai `ContentStore` et un vrai LocalStorage sous `t.TempDir()`, plus une base SQLite de test pour le registre ad hoc.

## Comportements observés

- `Get`, `Exists` et `Verify` lisent correctement un contenu préexistant au chemin OID legacy lorsque le mapping n'a aucune ligne.
- `Put` écrit ensuite le même type de pointeur à une clé générationnelle distincte, après réservation DB; le mapping courant permet aux méthodes du vrai `ContentStore` de retrouver ce contenu.
- Un second `Put` au même OID publie une seconde clé et bascule le mapping; supprimer directement l'ancienne clé physique après la bascule ne change pas les octets lus par `ContentStore.Get`.
- Un contenu de même taille mais de mauvais hash produit l'échec attendu du `hashingReader` de Forgejo; le mapping précédent et son contenu restent actifs. La nouvelle ligne pending demeure pour une éventuelle politique de reprise/nettoyage.

## Ce que cela n'établit pas

Le protocole est entièrement porté par l'adapter de test : `ContentStore` reste inchangé et `NewContentStore()` continue d'utiliser `storage.LFS` directement. La configuration de cet adapter et les tables du test ne sont reliées à aucun serveur Forgejo. Il n'y a pas de test d'endpoint LFS, transfert Git LFS, URL signée, migration, upload HTTP, association `LFSMetaObject`, stages de PR, import/mirror, GC ou suppression de dépôt.

Deux méthodes illustrent une limite bloquante de l'interface actuelle : `Delete(path)` reçoit seulement le chemin OID, sans génération/operation ID; l'adapter refuse ce delete logique pour éviter d'enlever une publication active. `IterateObjects` est refusé, car l'interface n'indique pas comment énumérer les clés actives et les générations retraitées sans supprimer à tort. Les URL sont réécrites par le test mais ne peuvent pas être exercées avec LocalStorage (`URL` non pris en charge). Une solution complète exige donc un seam typé de résolution/d'abandon par génération ou une transformation des call sites de suppression et GC; le mapping seul ne rend pas `ObjectStorage` drop-in.

Le verrou d'écriture de l'adapter n'est que dans le processus de test; pas de concurrence multi-instance, dialecte PostgreSQL/MySQL, crash de processus, S3/MinIO, récupération pending ni rétention. Les stages erronés restent présents après échec. Les méthodes et données d'essai ne sont pas du code produit. Aucun patch Forgejo n'est adopté/copié au produit; fork et coût upstream restent indécidés.
