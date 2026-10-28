# POC-039 — renouvellement et arrêt d’un stream LFS avec lease expirant

Date locale : 24 octobre 2026. Référence Forgejo : `v17.0.0`, commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`. Répertoire scratch déjà très modifié : `/tmp/forgejo-v17-base-cas-poc` (HEAD sur cette révision, état Git dirty). Aucun patch n’a été copié vers le dépôt produit, soumis ou déployé. Le `CONTRIBUTING.md` de Forgejo indique que le projet n’accepte pas de travaux écrits par un agent; cette expérience reste donc locale et ne constitue pas une proposition upstream.

## Changement expérimental

Dans `services/lfs/server.go`, le seam `streamVersionedContent` reconnaît désormais une capacité optionnelle sur l’objet ouvert : `RenewLFSReadLease(ctx)` et `LFSReadLeaseRenewalInterval()`. Un wrapper démarre un ticker, borne chaque appel de renouvellement par le contexte, et garde une seule fermeture de l’objet. Si le renouvellement échoue, le wrapper mémorise l’erreur, ferme l’objet pour interrompre la lecture et fait remonter cette erreur à `io.CopyN`. Le chemin legacy et les objets qui n’implémentent pas cette interface restent inchangés.

Le nouveau test de service utilise le protocole SQL du POC-038, un fichier SQLite et deux moteurs XORM séparés : l’un renouvelle le lease, l’autre tente atomiquement le claim GC, avec horloge logique contrôlée. Le test vert montre qu’un renouvellement avant l’échéance empêche le claim à l’ancienne échéance; le cas adverse laisse le GC gagner après expiration, puis vérifie que le renouvellement est refusé et que l’objet est fermé.

Deux tests supplémentaires traversent le vrai `DownloadHandler` et `LFSDelete` dans le routeur d’intégration Forgejo du scratch. Un objet de fixture limite les lectures à quatre octets et bloque le second read. Avec renouvellement accepté, la réponse complète arrive et la fermeture libère le lease puis supprime le blob retiré. Avec renouvellement refusé, le handler ferme le lecteur après les quatre premiers octets. Le statut HTTP `200` et le `Content-Length` complet ont déjà été envoyés avant le refus; le test vérifie donc le préfixe de quatre octets et le corps tronqué, pas une réponse tardive `409`. Le logger du harness affiche un `FATAL ERROR` pour le `log.Error` attendu, alors que le test sort 0.

## Résultats reproductibles

Toolchain : Go `1.27.3 linux/arm64`, `GOOS=linux`, `GOARCH=arm64`, `CGO_ENABLED=1`; `GOTOOLCHAIN=local`, `GOPROXY=off`, `GOSUMDB=off`.

- Avant le seam, les deux tests rouges signalaient explicitement l’absence de renouvellement et l’absence d’arrêt après refus (`red.log`). Cette première version du test utilisait une horloge mémoire; les tests verts ont ensuite été renforcés avec les deux connexions SQLite.
- Les deux cas SQLite ciblés passent 2/2 (`sqlite-focused.log`), avec l’échéance stricte utilisée par les helpers POC-038.
- Le paquet `services/lfs` complet passe avec les tags du téléchargement générationnel et du POC-039 (`package-final.log`, 1.144 s).
- Les deux tests service passent sous `go test -race` (`race.log`), sans diagnostic de course.
- Les deux nouveaux tests de route passent ensemble (`route-renewal-final.log`), puis avec `-count=5` (`route-renewal-repeat.log`). Le test de route POC-037 préexistant passe aussi après ce changement (`route-regression.log`, 1/1).
- `gofmt -d` est vide et `git diff --check` sur les chemins scratch modifiés est propre; résultats dans `gofmt-diff.log` et `whitespace.log`.

Les premiers essais de route sont conservés dans `route-renewal.log`: ils ont révélé une table de fixture non créée et une restauration de `storage.LFS` dans le mauvais ordre. Le test a été corrigé pour synchroniser la table et laisser le fixture remettre le stockage en place avant le teardown; les échecs initiaux ne démontraient pas un défaut produit.

## Limites déterminantes

Le renouvellement SQL exécuté pendant les deux tests de service est réel pour le modèle SQLite du POC, mais horloge et stockage sont des fixtures locales. Les tests de route utilisent eux aussi un adaptateur test-only; son `RenewLFSReadLease` n’écrit pas dans le registre SQLite expirant. Ces preuves établissent séparément le CAS SQLite, le wrapper du stream et son câblage au vrai handler, pas encore leur composition avec un adaptateur Forgejo durable de bout en bout.

Aucun writer ni backend de production Forgejo n’implémente cette capacité. L’interface `storage.Object` ne garantit pas que `Close()` interrompe un read réseau bloqué; l’adaptateur doit fournir ce comportement. Le code ne traite pas le client qui se déconnecte, les différences d’horloge entre hôtes, le budget total du transfert, les pannes/retries d’un backend distant, S3/MinIO, un worker GC multi-processus, Doctor, migration, dump/restore ni `git-lfs`. Après que l’en-tête est envoyé, le serveur ne peut plus convertir un refus en statut HTTP; le client doit détecter un corps plus court que `Content-Length`. Le logger d’intégration note cette erreur attendue.

Le scratch contient de nombreuses modifications antérieures; son diff total n’est pas la taille de ce lot. Les empreintes de `source-sha256.txt` se rapportent aux fichiers explicitement listés au moment du run; le fichier `lfs_delete_during_download_generation_poc_test.go`, qui définit une partie du fixture de route, n’y figurait pas. POC-040 étend le schéma de ce fixture et fournit l’inventaire de sources complet pour les nouveaux tests; ne traite pas le manifeste de POC-039 comme une empreinte exhaustive des dépendances de test. Rien ne justifie à ce stade un fork ou une adoption upstream.

Voir les sources, versions, logs et empreintes dans ce répertoire. `SHA256SUMS` couvre les preuves locales sauf lui-même.

## Commandes exactes

Les commandes suivantes ont été lancées depuis la racine du scratch avec `GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off` :

```sh
# Rouge TDD pré-implémentation; première fixture à horloge mémoire
/tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_read_lease_stream' ./services/lfs -run '^TestPOC(StreamRenewsLeaseWhileStorageReadIsBlocked|RefusedLeaseRenewalClosesTheActiveStream)$' -count=1 -v

# Cas finaux sur SQLite à horloge simulée et deux moteurs
/tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_download forgejo_poc_lfs_read_lease_stream' ./services/lfs -run '^TestPOC(StreamRenewsLeaseWhileStorageReadIsBlocked|RefusedLeaseRenewalClosesTheActiveStream)$' -count=1 -v

# Package complet des POC de téléchargement et leases
/tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_generation_streamed_download forgejo_poc_sqlite_generation_download forgejo_poc_lfs_read_lease_stream' ./services/lfs -count=1

# Tests ciblés sous le détecteur de courses
/tmp/forgejo-v17-toolchain/go/bin/go test -race -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_download forgejo_poc_lfs_read_lease_stream' ./services/lfs -run '^TestPOC(StreamRenewsLeaseWhileStorageReadIsBlocked|RefusedLeaseRenewalClosesTheActiveStream)$' -count=1 -v

# Route DownloadHandler/LFSDelete, deux comportements puis répétition x5
/tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' ./tests/integration -run '^TestPOCLFSRoute(RenewsLeaseDuringBlockedStorageRead|StopsAfterLeaseRenewalIsRefused)$' -count=1 -v
/tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' ./tests/integration -run '^TestPOCLFSRoute(RenewsLeaseDuringBlockedStorageRead|StopsAfterLeaseRenewalIsRefused)$' -count=5

# Régression de la route de rétention POC-037
/tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' ./tests/integration -run '^TestPOCLFSDeleteRouteDuringResolvedDownloadKeepsTheStreamReadable$' -count=1 -v
```
