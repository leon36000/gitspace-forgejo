# POC-037 — Conserver un GET LFS déjà résolu pendant la suppression

Date d'exécution : 24 octobre 2026, heure locale de Montréal. Référence Forgejo officielle : `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch `/tmp/forgejo-v17-base-cas-poc` contient les expérimentations antérieures; son état et les empreintes des sources examinées sont consignés dans `scratch-source-sha256.txt`; the bounded adapter/source review is in `source-review.txt`. Go `1.27.3 linux/arm64`; téléchargements de modules et vérification de sommes désactivés (`GOPROXY=off`, `GOSUMDB=off`).

## Question et rouge

POC-036 avait rendu le GET versionné, mais ne retenait pas physiquement l'ancienne génération. Le test rouge entrelace le GET authentifié et le vrai POST de Settings `LFSDelete`: après résolution, la suppression officielle retire le mapping et le blob avant `OpenLFSGeneration`. Le POST retourne `303`, puis le GET retourne `409 Conflict` sans contenu au lieu de lire les octets de la plage demandée. Source et sortie : `unleased-red-test.go`, `red-unleased-route.log`.

Le point d'entrelacement est déterministe et synchrone, dans le test de route. Ce n'est pas une mesure d'une course aléatoire.

## Adaptateur avec lease de lecteur

Dans le scratch uniquement, une fixture SQLite réserve dans la transaction de résolution un lease opaque lié à la génération. L'adaptateur de test retire le mapping logique lors du POST réel, passe la génération à `retired_pending_delete` et diffère l'effacement physique tant qu'un lease actif existe. Le GET ouvre exactement la génération du lease; `Close` libère le lease et déclenche la suppression différée si c'était le dernier lecteur.

Le test traverse les vrais routeurs Forgejo pour `DownloadHandler` et `LFSDelete`, mais utilise la table de mapping/lease et l'adaptateur `LocalStorage` propres à cette expérimentation. Il reçoit `206`, `Content-Range: bytes 3-6/<taille>` et le contenu attendu. Il confirme que la ligne de mapping et l'association de dépôt ont disparu, le lease est libéré, puis la génération physique est supprimée. Aucun fallback legacy n'est appelé. La signature du seam du `DownloadHandler` dans `services/lfs/server.go` a aussi été renommée en jeton opaque plutôt qu'en clé de stockage; ce fichier modifié est exclusivement dans le scratch.

## Vérifications

La commande combinée des POC-031 à -037 sélectionne neuf fonctions d'intégration (les scénarios de hash/taille ont des sous-cas), toutes réussies en `9.307s`; voir `integration-final.log`. Le paquet `./services/lfs` avec tags SQLite et POC a réussi en `0.233s`; voir `services-lfs-package.log`. Le rouge volontaire est conservé séparément; le test vert précis figure dans la suite combinée. Les commandes sont :

```sh
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_sqlite_generation_upload_route' \
  ./tests/integration \
  -run '^TestPOCLFS(AuthenticatedUploadPromotesSQLiteGenerationAndReadsItBack|UploadRetryPromotesDurablePendingGeneration|UploadSurvivesProcessRestart|InvalidUploadRejectsStageThenFreshAttemptRecovers|SettingsDeleteResolvesCurrentGeneration|SettingsDeleteCanRetryAfterStorageFailure|BatchDownloadURLUsesActiveGenerationAndOmitsAuthorization|DownloadRouteUsesResolvedGenerationAndRejectsPromotionRace|DeleteRouteDuringResolvedDownloadKeepsTheStreamReadable)$' \
  -count=1 -v

GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_generation_streamed_download forgejo_poc_sqlite_generation_download' \
  ./services/lfs -count=1
```

Le test rouge s'exécute seul avec le tag additionnel `forgejo_poc_lfs_unleased_red`; il est censé terminer en échec, précisément parce que la réponse réelle est `409` au lieu du `206` attendu. `gofmt` et `git diff --check` ciblés sur les deux fichiers expérimentaux réussissent.

## Limites et suite

Le lease SQLite et son `sync.Mutex` sont une fixture mono-processus. Il n'y a pas de bail qui expire, de processus de réconciliation des leases orphelins, de reprise après crash du lecteur, ni de coordination entre instances Forgejo. Un crash après la création du lease peut donc laisser une ligne active et différer indéfiniment le GC. Le test ne couvre ni S3/MinIO, ni URL batch/presign pendant la rétention, ni Doctor/GC global, ni migration/dump/restauration, ni client `git-lfs`. Il ne prouve pas que tous les chemins de suppression respectent ce contrat.

Conclusion limitée : un jeton de génération doit retenir le blob au moins jusqu'à la fin du flux; vérifier ensuite les décorateurs de storage, l'expiration/récupération des leases et la sûreté interprocessus, puis raccorder Doctor/GC et migrations/dumps. Aucun patch n'est proposé à Forgejo, aucun code n'est copié dans le produit, aucune instance n'est modifiée. Aucun agent LLM n'a participé et aucun gain comparatif n'est établi.
