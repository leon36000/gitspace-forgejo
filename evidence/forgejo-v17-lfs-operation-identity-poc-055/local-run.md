# Forgejo v17 — identité LFS pending immuable à travers les pages (POC-055)

Date : 25 octobre 2026. Base officielle Forgejo `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Travail effectué dans `/tmp/forgejo-v17-base-cas-poc`, scratch préexistant et sale; l’empreinte de la source réellement testée est dans [scratch-source-hashes.txt](scratch-source-hashes.txt). Aucun code Forgejo n’a été copié au dépôt produit, soumis ou déployé.

## Défaut reproduit et correction expérimentale

Le test rouge `TestLFSPendingMetaOperationIdentityIsImmutableAcrossPointers` a d’abord staged deux OIDs sous le même `OperationKey` avec des refs cibles différentes. L’ancienne fonction `StageLFSMetaObject` acceptait la seconde association; son invariant n’était donc pas stable entre pages. La trace capturée est dans [operation-identity-red.log](operation-identity-red.log).

Le scratch ajoute maintenant `LFSPendingMetaOperation`, un enregistrement DB de taille constante contenant la clé, le dépôt, la ref cible, la base et le candidat. `StageLFSMetaObject` enregistre cet en-tête et chaque association dans une transaction et refuse une identité divergente, même lorsque le second OID aurait été renvoyé par une autre page. La récupération paginée lit l’en-tête une fois par clé et par page, puis compare chaque ligne avant toute promotion de la page. Les associations d’une même opération présentes dans deux pages reprennent ainsi la même identité persistée.

La correction n’est pas portée au produit. Les changements scratch sont dans `models/git/lfs.go`, `models/git/lfs_test.go` et le récupérateur expérimental `services/pull/lfs_recovery.go`; les fichiers testés sont identifiés par SHA-256. Le registre xorm de ce scratch inclut le nouveau modèle; cela ne prouve pas une migration/backfill prête pour une instance existante.

## Vérifications observées

- Avant correction, le test d’identité échoue parce qu’aucune erreur n’est renvoyée pour la deuxième ref.
- Après correction, le test unitaire ciblé passe. Les quatre tests ciblés de récupération passent; le test paginé découpe déjà une opération commune sur deux pages.
- Les paquets complets `./services/pull` et `./models/git` passent avec `-tags sqlite`.
- Deux tentatives concurrentes de staging du même OperationKey avec des refs différentes, en SQLite fichier, donnent une seule association gagnante et une seule erreur; 10 répétitions passent sous `-race`.
- La promotion par ID, avec 16 goroutines en compétition, passe également 10 répétitions sous `-race` avec le modèle d’opération ajouté.
- `gofmt -d` n’affiche aucun changement et un scanner ciblé ne trouve pas d’espaces finaux dans les quatre fichiers. `git diff --check` passe pour les deux fichiers modèles suivis; Git ne contrôle pas par cette commande les fichiers service non suivis.

Commandes Go (avec `GOTOOLCHAIN=local GOPROXY=off`, binaire `/tmp/forgejo-v17-toolchain/go/bin/go`) :

```sh
go test -count=1 -v -timeout=20m -tags sqlite ./models/git \
  -run '^TestLFSPendingMetaOperationIdentityIsImmutableAcrossPointers$'
go test -count=1 -v -timeout=20m -tags sqlite ./services/pull \
  -run '^TestRecoverLFSPendingMetaObjects(AfterInterruptedPostReceive|DefersMissingCandidate|DoesNotPromoteFromUnrelatedRef|PageIsBoundedAndResumable)$'
go test -count=1 -timeout=20m -tags sqlite ./services/pull
go test -count=1 -timeout=20m -tags sqlite ./models/git
go test -race -count=10 -timeout=20m -tags sqlite ./models/git \
  -run '^TestConcurrentLFSPendingMetaOperationIdentityRejectsConflictingStage$'
go test -race -count=10 -timeout=20m -tags sqlite ./models/git \
  -run '^TestConcurrentLFSPendingPromotionByIDIsIdempotent$'
```

Les journaux correspondants sont conservés ici : `model-identity-focused.log`, `service-focused.log`, `services-pull.log`, `models-git-final.log`, `stage-identity-race-repeat.log` et `promote-by-id-race-repeat.log`. `models-git-before-final-test.log` archive un run complet antérieur à l’ajout du test de staging concurrent. `poc-054-promote-race-repeat.log` est conservé comme résultat historique du lot précédent; le run actuel est `promote-by-id-race-repeat.log`.

## Limites et suite

La table d’identité doit être migrée et les lignes pending historiques doivent être backfillées avant qu’un tel chemin puisse être activé sur une base déjà peuplée; le chemin paginé échoue fermé si l’enregistrement manque. Un insert SQL direct peut contourner l’API de staging. Les en-têtes d’opération restent après promotion/discard; aucune rétention/nettoyage n’est implémenté. La concurrence a seulement été exercée avec SQLite fichier sur le même hôte, pas entre processus distincts, hôtes, PostgreSQL ou MySQL. Une erreur d’insert de clé concurrente peut nécessiter la reprise de l’appelant; les tests prouvent un seul gagnant et une seule erreur, pas sa classification ou son ordonnancement.

Le check de ref Git précède toujours la promotion DB sans transaction commune; une course de ref demeure. Le récupérateur paginé et le modèle restent scratch, sans caller/worker de production, fairness, reset durable du curseur, rétention, nettoyage ou audit exhaustif des writers. Les faits POC-051/053/054 restent expérimentaux. Le prochain lot doit spécifier le checkpoint/cycle durable, son reset pour revisiter les pending différés, la rétention des en-têtes, la compatibilité/backfill et la politique de reprise avant tout worker ou décision de fork.

Aucun LLM/AgentOS, serveur Forgejo de production, secret ou service externe n’a été utilisé; aucun benchmark de valeur produit n’est établi.
