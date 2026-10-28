# POC-040 — composer la route LFS avec le lease SQLite et le claim GC

Date : 24 octobre 2026. Forgejo officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; scratch `/tmp/forgejo-v17-base-cas-poc`, HEAD détaché sur cette révision, worktree dirty avant et après le lot. `CONTRIBUTING.md` de Forgejo indique que le projet n’accepte pas le code ou la documentation partiellement écrits par IA. Cette preuve reste locale; aucun code n’a été copié dans le produit, soumis ou déployé.

## Ce qui a été relié

Le nouveau test d’intégration traverse le vrai `DownloadHandler`/`streamVersionedContent`, la route `LFSDelete`, et l’objet `storage.Object` sur la configuration d’intégration Forgejo et `LocalStorage`. Le fixture de route utilise une seule ligne `poc_route_lfs_read_lease` avec échéance, dans le même fichier SQLite que `poc_route_lfs_generation`. L’interface GC crée un second moteur XORM sur exactement le même DSN; le test asserte que les deux moteurs sont distincts et que les DSN concordent.

Le renouvellement est un `UPDATE` SQL conditionnel sur la même ligne de lease : le lease doit être actif et non expiré, et la génération correspondante doit rester `active` ou `retired_pending_delete`. Le claim GC passe la génération `retired_pending_delete` à `delete_claimed` si aucune ligne active de lease n’a `expires_at_ms > now`. Le renouvellement transmet son contexte à une session XORM; aucun scénario de verrou SQL long ou d’annulation de contexte n’est testé.

Deux ordres déterministes utilisent des canaux et une horloge logique :

1. À `now=99`, le lease expire à `100`; le stream bloque à son second read. Le renouvellement capture `99` et écrit `1099`. Puis le claim GC à `100` échoue, le handler complète le corps et la fermeture libère le lease puis permet la suppression de la génération.
2. Le renouvellement capture `99` avant une barrière. Le test passe l’horloge à `100`, puis le second moteur GC réclame la génération. Après libération de la barrière, l’`EXISTS` sur l’état de génération fait échouer le renouvellement, même si son horloge capturée est antérieure à l’expiration. Le handler s’arrête après le premier bloc de quatre octets; le statut `200` et `Content-Length` complet étaient déjà envoyés. Le test vérifie ensuite `released`, finalise manuellement le claim GC et relit `deleted`/blob absent.

La route confirme avant les deux cas que l’API `LFSDelete` a mis la génération en `retired_pending_delete` pendant que le lease la protège. La première exécution rouge laissait le renouvellement en no-op : le GC réclamait alors la génération à l’ancienne échéance. L’extrait est dans `red.log`; les réussites sont dans `current.log`, `repeat.log` et `race.log`. Le test de route POC-039 et la régression POC-037 passent aussi (`regression.log`).

## Résultats reproductibles

Go `1.27.3 linux/arm64`, `GOOS=linux`, `GOARCH=arm64`, `CGO_ENABLED=1`, avec `GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off`. Le harnais choisit `tests/sqlite.ini`; la version SQLite n’a pas été interrogée séparément. Forgejo enregistre un `log.Error` attendu quand le stream est tronqué; le test logger imprime alors `FATAL ERROR`, mais `go test` termine avec `PASS`/code 0.

Commandes exécutées depuis la racine du scratch :

```sh
# TDD rouge, avant l’implémentation du CAS SQL
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' ./tests/integration -run '^TestPOCSQLiteRouteRenewalPreventsClaimAtOldExpiry$' -count=1 -v

# Deux cas finaux, répétés deux fois
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' ./tests/integration -run '^TestPOCSQLiteRoute(RenewalPreventsClaimAtOldExpiry|StopsWhenExpiredLeaseWasClaimedByGC)$' -count=2 -v

# Même paire sous le détecteur de courses
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test -race -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' ./tests/integration -run '^TestPOCSQLiteRoute(RenewalPreventsClaimAtOldExpiry|StopsWhenExpiredLeaseWasClaimedByGC)$' -count=1 -v

# Régressions POC-037 et POC-039 après extension du schéma de lease du fixture
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' ./tests/integration -run '^Test(POCLFSRoute(RenewsLeaseDuringBlockedStorageRead|StopsAfterLeaseRenewalIsRefused)|POCLFSDeleteRouteDuringResolvedDownloadKeepsTheStreamReadable)$' -count=1 -v
```

## Portée et limites

Les tests passent par le vrai handler du binaire d’intégration, mais renouvellement SQL, tables d’échéance, claim, fixture de stockage et finalisation GC sont des adaptateurs/test helpers; Forgejo de production n’implémente pas cette capacité. Le worker GC est appelé manuellement après le stream, pas exécuté comme processus durable. Les canaux imposent les deux ordres de commit; ils ne simulent pas des écritures réellement simultanées, une erreur/verrou SQLite, ni un moteur SQL différent.

La fermeture/interruption d’un `Read` bloqué est fournie par l’objet fixture. `httptest.ResponseRecorder` observe un handler à corps raccourci; aucun client HTTP réel ou `git-lfs` n’observe une EOF prématurée. L’horloge métier est logique; la période de cinq secondes reflète aussi le timeout de renouvellement du wrapper. Sont absents : budget total, client disconnect, erreurs/retry backend, multi-hôte et dérive d’horloge, S3/MinIO, GC multi-processus, arrêt/reprise du worker, Doctor, migrations, dump/restore et toute preuve d’intégration de production.

La revue read-only indépendante confirme la course ciblée fermée dans ce modèle SQLite et demande de garder ces limites explicites. Aucun fork, patch upstream ou adoption n’est justifié par ce POC.

Les sources complètes et leurs empreintes sont dans `source-sha256.txt`; versions dans `tool-versions.txt`. `SHA256SUMS` couvre les preuves locales, sauf lui-même.
