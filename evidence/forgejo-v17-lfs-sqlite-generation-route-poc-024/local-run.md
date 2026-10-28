# POC-024 — route LFS authentifiée avec adapter SQLite générationnel

Date : 24 octobre 2026. Scratch Forgejo officiel v17.0.0 à `/tmp/forgejo-v17-base-cas-poc`, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571`. Go `1.27.3 linux/arm64`. Aucun changement au dépôt produit ni déploiement.

## Question et trajet

POC-023 raccordait SQLite à `streamVersionedContent`, mais appelait directement le helper et ne traversait pas l’authentification ou la route. Ce POC ajoute seulement un test d’intégration local sous le build tag `forgejo_poc_sqlite_generation_route`.

Dans le harnais d’intégration Forgejo configuré avec SQLite et stockage local, le test remplace temporairement `storage.LFS` par un adapter test-only qui implémente `storage.ObjectStorage`. La fixture appelle la route LFS authentifiée existante. Le vrai `ContentStore` écrit les octets à une clé physique de génération unique; l’adapter enregistre la ligne `pending` dans SQLite, vérifie taille et SHA-256 sur l’objet stocké, puis promeut atomiquement la génération vers le mapping courant. La vraie route `DownloadHandler` relit ensuite le contenu par ce mapping. La variable globale est restaurée avant la fin du test.

## Résultat observé

La requête authentifiée avec `Range: bytes=3-6` répond `206 Partial Content`, `Content-Range: bytes 3-6/10` et corps `3456`. Le readback DB confirme une génération courante `1`, une clé sous `generation/<oid>/1/...` et une rangée correspondante en état `active`. La sortie du test montre le passage par `lfs.DownloadHandler`.

## Commandes et résultat

Depuis `/tmp/forgejo-v17-base-cas-poc` :

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go version
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route' ./tests/integration -run '^TestPOCLFSAuthenticatedDownloadRouteReadsSQLiteGenerationMapping$' -count=1 -v
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/gofmt -d tests/integration/lfs_sqlite_generation_route_poc_test.go
git diff --check -- tests/integration/lfs_sqlite_generation_route_poc_test.go
```

Le test d’intégration sélectionné passe : `ok forgejo.org/tests/integration 0.582s`. Gofmt et le diff-check n’émettent aucun diagnostic. Le journal, la source exacte du test et les hashes sont dans ce dossier.

## Portée et limites

Cette exécution traverse Forgejo local, la vraie route HTTP, l’authentification, le vrai `ContentStore`, SQLite de test et LocalStorage. Elle ne démontre pas un client Git LFS externe, un backend S3/MinIO, un crash, un deuxième processus concurrent, migration, nettoyage/GC ou une suppression. `Delete` logique et `IterateObjects` sont explicitement refusés par le fixture. Le handler continue d’utiliser `legacyLFSContentStore`; le test prouve le mapping au niveau du storage `Open`, pas un jeton de génération lié au couple `Resolve/Open` dans la route. L’assignation globale de storage n’est qu’un seam de test et n’est pas proposée comme configuration de production. Aucun code de Forgejo n’a été copié au produit ou adopté; coût upstream et revue exhaustive des writers/readers restent ouverts.

Suite proposée : exercer un remplacement du contenu dans une requête réelle, préciser la sémantique de lecture quand le mapping bascule, puis auditer tous les call sites de suppression, URL et GC. Garder ce protocole en scratch jusqu’à ces vérifications.
