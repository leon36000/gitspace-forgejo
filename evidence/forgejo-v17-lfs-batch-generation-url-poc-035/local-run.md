# POC-035 — URL batch LFS liée à la génération active

Date du lot : 24 octobre 2026 (date de l’environnement de mission). Scratch Forgejo officiel v17.0.0, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571`; le checkout `/tmp/forgejo-v17-base-cas-poc` contient les POC précédents non adoptés. Go `1.27.3 linux/arm64`; `GOPROXY=off` et `GOSUMDB=off`.

## Question et périmètre

Le vrai endpoint Forgejo `POST /{owner}/{repo}.git/info/lfs/objects/batch` peut-il obtenir une URL de téléchargement à partir de la clé de la génération active, et retirer l’en-tête d’autorisation quand l’URL directe est fournie?

Le test sous build tag appelle le routeur et `BatchHandler` réels. Il crée le blob et les métadonnées LFS avec les helpers d’intégration, puis remplace `storage.LFS` par un adaptateur de test : le mapping courant est stocké dans SQLite et les octets dans `LocalStorage`. Le backend URL de fixture enregistre la clé demandée et renvoie une URL déterministe portant explicitement une fausse signature. `ServeDirect` est activé pour ce seul test.

## Observation

La requête batch authentifiée retourne HTTP 200, un objet sans erreur et une action `download`. L’adaptateur reçoit exactement `generation/<oid>/1/route-1`; le `href` JSON contient cette clé et l’URL de fixture. L’action ne contient pas l’en-tête `Authorization`. Cela vérifie que `pocRouteLFSStorage.URL` traduit le chemin LFS logique vers la génération active et que `buildObjectResponse` omet le secret d’authentification sur le chemin direct.

La commande combinée sélectionne sept fonctions d’intégration : le nouveau batch URL et six tests POC-031 à POC-034 (upload, reprise, refus hash/taille et suppression). Les sept passent, dont les sous-cas de refus. La sortie contient des messages `FATAL ERROR` du testlogger pour les `log.Error` intentionnels des scénarios négatifs; les assertions attendent leurs statuts et la commande termine avec succès.

## Commande

```sh
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_sqlite_generation_upload_route' \
  ./tests/integration \
  -run '^TestPOCLFS(AuthenticatedUploadPromotesSQLiteGenerationAndReadsItBack|UploadRetryPromotesDurablePendingGeneration|UploadSurvivesProcessRestart|InvalidUploadRejectsStageThenFreshAttemptRecovers|SettingsDeleteResolvesCurrentGeneration|SettingsDeleteCanRetryAfterStorageFailure|BatchDownloadURLUsesActiveGenerationAndOmitsAuthorization)$' \
  -count=1 -v
```

Le log exact est `final-test.log`. `scratch-test-source.go` conserve le nouveau test tagué. `scratch-source-sha256.txt` identifie les cinq fichiers pertinents du scratch; les versions d’outils et le commit sont dans `tool-versions.txt`.

## Limites

La signature, le service de stockage URL et le mapping de génération sont des fixtures de test; aucun appel MinIO/S3 ou URL distante réelle n’a eu lieu. L’endpoint construit le lien, mais le client ne le suit pas : le `DownloadHandler` réel n’est pas encore relié au mapping `Resolve/Open`. Le POC ne vérifie ni URL expirée/révocation, ni échec de presign, ni fallback, ni concurrence avec une promotion/suppression. Il ne modifie aucun code produit Forgejo et ne justifie pas un fork.

Suite utile : brancher `Resolve/Open` au handler GET dans un test d’intégration isolé; vérifier le lien vers la même génération après promotion concurrente, puis couvrir refus/fallback URL, Doctor/GC et les call sites restantes. Le scratch demeure expérimental; ne pas adopter son code sans preuve d’intégration et coût upstream.
