# POC-033 — refus des stages LFS invalides et retry corrigé

Date : 24 octobre 2026. Forgejo scratch : tag `v17.0.0`, commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch reste fortement modifié par des POC antérieurs. Le lot ne modifie que son test d'intégration tagged; aucun code de production Forgejo n'est modifié.

## Parcours et résultat

Le test envoie au vrai routeur d'intégration les deux charges invalides suivantes, par le vrai `UploadHandler` et le vrai `lfs.ContentStore` : contenu de même longueur mais SHA-256 différent du pointeur, puis corps plus court que la taille déclarée. Les deux réponses HTTP sont `422 Unprocessable Entity`. Le seam de stockage test-only inscrit le stage sous un identifiant d'opération propre, observe la validation taille/hash du `ContentStore`, passe la génération à `rejected` et supprime sa clé physique immuable. Le test confirme l'absence de mapping `current`, de blob stage et de `LFSMetaObject`.

Chaque sous-test renvoie ensuite une demande corrigée avec un nouvel OperationID. Le stage devient la génération 2, active, le PUT répond `200`, et le vrai GET `DownloadHandler` avec `Range` renvoie `206` et les octets attendus. Une suite combine aussi les deux tests de succès/retry de POC-031 et le crash/reprise interprocessus POC-032 : quatre tests de premier niveau passent (le test hash/size contient deux sous-tests).

La suppression après rejet et la sémantique `rejected` sont exclusivement dans l'adaptateur de test. Cela ne prouve pas un nettoyage sûr pour un stockage objet distant, une panne pendant la suppression, une reprise/GC automatique, ni un mécanisme de production. Le harnais utilise toujours ses fixtures locales; la preuve n'exerce pas un client `git-lfs`.

## Commande et observations

Forgejo `v17.0.0` au commit `0d791421f85ca13a63b665053d81ea765fbbc571`, Go `1.27.3 linux/arm64`, XORM `v1.4.1`, pilote SQLite `v1.14.52`.

```sh
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_upload_route' \
  ./tests/integration \
  -run '^TestPOCLFS(AuthenticatedUploadPromotesSQLiteGenerationAndReadsItBack|UploadRetryPromotesDurablePendingGeneration|UploadSurvivesProcessRestart|InvalidUploadRejectsStageThenFreshAttemptRecovers)$' \
  -count=1 -v
```

Le dernier run sort avec code 0; journal intégral `final-test.log`. Les logs `testlogger.go:recordError() FATAL ERROR` accompagnent les refus attendus parce que le handler appelle `log.Error`; `modules/testlogger/testlogger.go` laisse l'appel `t.Errorf` correspondant commenté. Les assertions HTTP et l'exit code du processus constituent le résultat. `gofmt -d` est vide; le scan ne trouve ni espaces finaux ni marqueurs de conflit.

Une première composition des tests avait réutilisé des OperationID fixes dans les tables POC persistantes du harnais, et échouait sur conflit d'identité. Le test emploie maintenant un UUID par tentative; les deux scénarios passent lorsqu'ils sont isolés et dans la commande combinée finale. Il s'agissait d'une collision de fixture, pas d'un comportement du handler Forgejo.

## Limites et suite

Le test ne traite pas les interactions de suppression normale, Doctor, GC global, URL directe, `Resolve/Open`, migrations/dump ou l'inventaire de tous les writers; il ne couvre pas MinIO/S3, concurrence inter-hôte, données durables de l'application Forgejo ou coût upstream. La prochaine revue doit borner ces call sites avant de concevoir un seam typed et chiffrer toute divergence à maintenir. Aucun fork ou changement de licence n'est proposé.
