# POC-017 — un effacement périmé peut toucher une nouvelle publication

Date : 24 octobre 2026. Forgejo `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; test diagnostique tagué dans `/tmp/forgejo-v17-base-cas-poc`. Go `1.27.3 linux/arm64`, SQLite de test et stockage local temporaire. Aucun code Forgejo n’est copié au produit.

## Expérience

Le contrat `modules/storage/storage.go::ObjectStorage` expose `Delete(path)` sans génération, version d’objet ou précondition conditionnelle. L’implémentation locale supprime par chemin (`modules/storage/local.go::LocalStorage.Delete`); MinIO appelle `RemoveObject` avec des options vides (`modules/storage/minio.go::MinioStorage.Delete`). Un décorateur test retarde le premier appel avant son opération sous-jacente. Le scénario exécute ensuite un second delete, publie à nouveau le même contenu adressé par son OID, puis laisse l’ancien appel reprendre. Le test `TestStaleLFSDeleteCanRemoveNewPublication` passe en observant que l’appel périmé supprime l’objet récemment republié au même chemin.

## Vérification

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify poc_lfs_stale_delete' ./services/repository \
  -run '^TestStaleLFSDeleteCanRemoveNewPublication$' -v
```

Résultat : `PASS`, `ok forgejo.org/services/repository 0.304s`; trace : [`stale-delete.log`](stale-delete.log). `gofmt -d` et `git diff --check` sont vides. Les empreintes SHA-256 du test et des trois interfaces/implémentations auditées sont dans [`scratch-source-sha256.txt`](scratch-source-sha256.txt).

## Sens et limites

Le test montre qu’un protocole DB qui expire/réessaie une réservation sans contrôler un appel de stockage déjà en vol peut supprimer une publication plus récente, si le backend laisse cet appel s’achever tardivement. **Il impose artificiellement cet ordre** : il n’établit pas que LocalStorage ou MinIO exécutent réellement une requête de suppression périmée après leur retour/cancellation, ni leur sémantique sous partition réseau. Aucun backend MinIO réel n’a été testé.

Avant un retry automatique, il faut prouver l’absence d’appel ancien en vol pour chaque backend ou ajouter une primitive de fencing/version conditionnelle que l’implémentation respecte, et tester les pannes ambiguës. L’OID seul n’est pas un jeton de génération, même si le contenu est immuable. Le scratch reste non adopté; aucune correction n’est proposée.
