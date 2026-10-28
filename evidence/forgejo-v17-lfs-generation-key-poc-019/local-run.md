# POC-019 — isolation physique par clé de génération (24 octobre 2026)

## Hypothèse

Si un nouvel objet de même OID est publié sous une clé physique immutable et différente, un ancien `Delete(oldKey)` peut terminer après cette publication sans retirer le contenu associé à `newKey`.

## Source et test

Scratch `/tmp/forgejo-v17-base-cas-poc`, Forgejo officiel v17.0.0 HEAD `0d791421f85ca13a63b665053d81ea765fbbc571`; aucun code scratch n’a été copié au produit. Le nouveau test tagué `forgejo_poc_lfs_generation` utilise le vrai `LocalStorage`: il écrit l’ancienne génération, retient un delete enveloppant juste avant l’appel de stockage, écrit le payload de la nouvelle génération sous un autre chemin, puis libère le delete ancien. Il vérifie que l’ancien chemin a disparu et que le nouveau chemin conserve exactement ses octets.

Commande depuis `/tmp/forgejo-v17-base-cas-poc` :

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_generation' ./modules/storage -run '^TestPOCStaleDeleteForOldGenerationLeavesNewGeneration$' -count=1 -v
```

Résultat : `PASS`, `forgejo.org/modules/storage`, 0.013 s, Go `1.27.3 linux/arm64`; `gofmt -d` est vide. Source exacte : `modules/storage/local_generation_delete_poc_test.go`, SHA-256 consigné dans `scratch-source-sha256.txt`.

## Ce que cela établit

Sur le backend LocalStorage et la couche physique, le chemin d’une génération ancienne peut être supprimé en retard sans supprimer un contenu écrit sous un chemin générationnel distinct. C’est une primitive qui rend le scénario POC-017 isolable par clé si les lecteurs suivent l’association DB courante.

## Limites

Le test ne crée ni registre OID→clé, ni transaction DB, ni association pending/active, ni endpoint/reader LFS. Il ne couvre pas MinIO/S3, la génération d’identifiants en concurrence, le partage inter-dépôts, les échecs après upload, le GC, le crash/reprise ou une migration de données. Sans mapping autoritaire et usage uniforme dans tous les call sites, ce résultat ne corrige rien dans Forgejo. Aucun code produit n’est modifié et le fork reste non adopté.
