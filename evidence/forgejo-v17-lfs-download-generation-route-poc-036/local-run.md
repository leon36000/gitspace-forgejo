# POC-036 — GET LFS lié à la génération résolue

Date du lot : 24 octobre 2026 (date locale de l’environnement de mission). Baseline Forgejo officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch `/tmp/forgejo-v17-base-cas-poc` est déjà modifié par les POC antérieurs; ses fichiers et versions examinés sont identifiés dans `scratch-source-sha256.txt` et `tool-versions.txt`. Aucun secret ni appel réseau n’est utilisé (`GOPROXY=off`, `GOSUMDB=off`).

## Question

Après qu’un GET LFS authentifié a résolu une génération donnée, le vrai `DownloadHandler` peut-il lire précisément cette génération et refuser si elle a été remplacée avant l’ouverture, au lieu de relire silencieusement le chemin OID courant?

## Changement expérimental dans le scratch

Avant le changement, le test rouge traverse le vrai routeur et reçoit bien la plage `206`, mais constate `resolveCalls=0`: `DownloadHandler` utilise uniquement le `ContentStore` legacy et ne consomme aucune génération résolue.

Dans la copie scratch seulement, `DownloadHandler` choisit maintenant une capacité optionnelle `ResolveLFSGeneration` / `OpenLFSGeneration` implémentée par un adaptateur de stockage; si elle manque, le chemin legacy reste actif. Le jeton opaque est la clé physique immuable de génération. `ErrLFSVersionStale` permet à l’adaptateur de signaler un changement de mapping entre résolution et ouverture; le handler répond alors `409`. La fixture vérifie le mapping SQLite à l’ouverture et ouvre la clé physique, jamais le chemin OID logique.

## Résultats

La requête GET réelle et authentifiée, avec `Range: bytes=3-6`, retourne `206`, `Content-Range: bytes 3-6/10` et `3456`. Le test observe un appel de résolution, un appel d’ouverture versionnée et aucun appel `Open` legacy.

Un hook de test promeut ensuite une nouvelle génération après `Resolve` mais avant `Open`. Le handler reçoit l’erreur `ErrLFSVersionStale`, répond `409 Conflict` et ne renvoie aucun octet. Enfin, retirer la ligne de mapping tout en gardant les métadonnées LFS donne `404`; aucun `OpenLFSGeneration` ou fallback legacy n’est appelé. Le journal rouge pré-changement et la source de test rouge sont conservés à côté du résultat final.

Après le changement, huit fonctions d’intégration ciblées des POC-031 à -036 passent ensemble (`./tests/integration`, 5.176 s), comprenant PUT/retry/redémarrage, refus hash/taille, suppression, URL batch et le nouveau GET versionné avec promotions périmées et mapping manquant. Le paquet complet `./services/lfs` passe aussi avec les tags POC/SQLite (`0.351 s`). `gofmt -d` et `git diff --check -- services/lfs/server.go` ne produisent aucune erreur.

## Commandes finales

```sh
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_sqlite_generation_upload_route' \
  ./tests/integration \
  -run '^TestPOCLFS(AuthenticatedUploadPromotesSQLiteGenerationAndReadsItBack|UploadRetryPromotesDurablePendingGeneration|UploadSurvivesProcessRestart|InvalidUploadRejectsStageThenFreshAttemptRecovers|SettingsDeleteResolvesCurrentGeneration|SettingsDeleteCanRetryAfterStorageFailure|BatchDownloadURLUsesActiveGenerationAndOmitsAuthorization|DownloadRouteUsesResolvedGenerationAndRejectsPromotionRace)$' \
  -count=1 -v

GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_generation_streamed_download forgejo_poc_sqlite_generation_download' \
  ./services/lfs -count=1
```

Les sorties exactes sont `integration-final.log` et `services-lfs-package.log`. La vérification ciblée du helper de stream est aussi conservée dans `services-lfs-final.log`.

## Portée et limites

Le changement dans `services/lfs/server.go` appartient exclusivement au scratch Forgejo; il ne modifie ni le dépôt produit ni une instance déployée, et n’est pas proposé à l’adoption. L’adaptateur, le registre SQLite et `LocalStorage` sont des fixtures. Aucun S3/MinIO, client `git-lfs`, vrai schéma Forgejo durable ou parcours multi-processus du handler modifié n’est exercé. Le hook couvre un seul ordre précis, résolution puis promotion puis ouverture; il ne teste pas une suppression simultanée après validation du mapping, ni la rétention de l’ancienne clé, les URLs expirées, Doctor/GC, les migrations ou dumps. Le passage du paquet `services/lfs` et huit fonctions d’intégration ciblées ne vaut pas test complet de Forgejo.

Il s’agit d’une preuve de faisabilité d’un seam optionnel, pas d’un coût de maintenance upstream, d’un fork décidé, d’un parcours LLM, ni d’un gain produit. Aucun AgentOS, RAG, mémoire ou modèle n’est intégré. Cargo reste sans licence choisie et `publish = false`.

## Prochaine tranche

Durcir le contrat du seam contre la suppression physique après `Resolve`, l’échec de lecture, les clés legacy, et les décorateurs qui perdent la capacité optionnelle; raccorder ensuite l’inventaire/Doctor/GC et le cycle migrations/dumps à une source de vérité unique. Revoir aussi le parcours d’admission webhook de bout en bout sans prétendre à une intégration AgentOS/LLM non constatée.
