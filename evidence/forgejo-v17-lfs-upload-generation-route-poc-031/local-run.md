# POC-031 — upload LFS HTTP vers une génération SQLite test-only

Date : 24 octobre 2026. Forgejo scratch : `/tmp/forgejo-v17-base-cas-poc`, tag `v17.0.0`, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571`. Go `1.27.3 linux/arm64`. Aucun changement du dépôt produit à Forgejo ni déploiement.

## Parcours exercé

Le nouveau fichier `tests/integration/lfs_sqlite_generation_upload_route_poc_test.go` est sous le seul build tag `forgejo_poc_sqlite_generation_upload_route`. Il substitue `storage.LFS` pendant la fixture par un adapter `storage.ObjectStorage` test-only qui réserve des générations dans des tables POC via `models/db.WithTx`, écrit les octets par clé immuable dans le véritable `storage.LocalStorage`, vérifie taille et SHA-256/OID puis promeut la génération dans une transaction. Les lecteurs de l’adapter résolvent uniquement l’association courante; une suppression sans clé de génération et l’inventaire GC sont refusés.

Le premier test envoie un PUT authentifié au vrai routeur d’intégration Forgejo. Le chemin observé est `routers/web/web.go:1899` → `services/lfs.UploadHandler` (`server.go:333`) → `lfs.NewContentStore` / `ContentStore.Put` (`modules/lfs/content_store.go:31,48`) → `storage.LFS.Save`. L’UploadHandler crée ensuite le `LFSMetaObject`. Un second PUT de l’OID déjà actif ne réserve pas une seconde génération. Le vrai GET/LFS `DownloadHandler` répond à `Range: bytes=2-5` avec `206`, `Content-Range: bytes 2-5/117` et `2345`.

Le second test injecte un arrêt déterministe après l’écriture et la vérification du stage, avant la transaction de promotion. Le premier PUT renvoie `500`; la base ne contient pas de mapping courant ni de `LFSMetaObject`, tandis que le stage `pending` et ses octets vérifiés restent présents. Le PUT rejoué réutilise le même `OperationID`, la même génération et la même clé; il promeut alors le mapping et crée le `LFSMetaObject`. Le GET avec range renvoie `206`, `Content-Range: bytes 14-18/120` et `01234`.

Le corps de `UploadHandler` utilisé par ce run est identique octet pour octet à celui du commit Forgejo v17.0.0 (`source-review.txt`). `services/lfs/server.go` reste toutefois modifié dans le scratch par des POC antérieurs sur le handler de téléchargement; son état Git sale et les fichiers de ce lot sont consignés dans `scratch-status.txt`. Le routeur, `ContentStore`, l’interface `storage.ObjectStorage` et `models/db.WithTx` utilisés ici sont aux fichiers v17.0.0 non modifiés. Ce lot n’ajoute qu’un test tagged; aucun fichier source de production Forgejo n’est modifié.

## Vérification

Commande finale, depuis `/tmp/forgejo-v17-base-cas-poc` :

```sh
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_upload_route' \
  ./tests/integration \
  -run '^TestPOCLFS(AuthenticatedUploadPromotesSQLiteGenerationAndReadsItBack|UploadRetryPromotesDurablePendingGeneration)$' \
  -count=1 -v
```

La commande est exécutée dans Bubblewrap avec `--unshare-net`, source scratch montée explicitement, cache Go temporaire et module cache en lecture seule. Résultat : les deux tests passent; `go test` sort avec code 0 (`final-test.log`). `gofmt -d` est vide et le scan du nouveau fichier trouve zéro espace final ou marqueur de conflit (`gofmt-check.log`, `style-check.txt`). Le journal de l’échec injecté inclut les appels `log.Error` normaux du handler et le texte `testlogger.go:recordError() FATAL ERROR`; Forgejo `modules/testlogger/testlogger.go:490-493` imprime ce diagnostic mais son appel `t.Errorf` est commenté. Le verdict du test vient donc du code de sortie 0 et des assertions de l’essai, pas de ce libellé de log.

Les fichiers `focused-test.log` et `all-poc-tests.log` préservent les essais exploratoires précédents; `final-test.log` correspond au dernier état du test source.

## Limites

La panne est une erreur injectée dans le même processus juste avant la promotion, pas un arrêt brutal. Le retry est séquentiel dans le même processus; il n’exerce pas la reprise interprocessus déjà modélisée séparément par POC-030. Les deux tables de générations et l’adapter restent expérimentaux. Ce test ne prouve ni concurrence multi-processus de la route, ni nettoyage du stage après un hash/size invalide, ni migration, GC/Doctor, suppression concurrente, URL versionnée, client `git-lfs`, S3/MinIO ou autres moteurs DB. Le swap de `storage.LFS` est un seam de test, pas une interface de configuration de production. Aucun LLM ni gain comparatif n’est exercé.

## Suite utile

Relier une interruption de processus au vrai PUT, puis relancer le service de test sur le même état pour récupérer et promouvoir le stage; vérifier aussi le refus/nettoyage après échec de hash et size. Ensuite couvrir suppression, Doctor/GC, URL et `Resolve/Open` avant de proposer un seam upstream. POC-031 ne justifie pas un fork et ne change pas la décision d’architecture.
