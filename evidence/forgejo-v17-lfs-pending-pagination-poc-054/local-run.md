# Forgejo v17 — récupération paginée des associations LFS pending (POC-054)

Date : 25 octobre 2026. Source officielle auditée : Forgejo `v17.0.0`, HEAD amont `0d791421f85ca13a63b665053d81ea765fbbc571`. L’expérience est restée dans le scratch préexistant `/tmp/forgejo-v17-base-cas-poc`, déjà très modifié; son HEAD est cette révision, mais ses fichiers ne sont pas propres. L’empreinte des fichiers évalués est donnée dans [source-hashes.txt](source-hashes.txt). Aucun code Forgejo n’a été copié dans le dépôt produit, soumis ou déployé.

## Hypothèse et surface examinée

Après POC-053, les associations pending pouvaient être récupérées sans limite de lignes par appel. Cette tranche ajoute au scratch une lecture keyset par identifiant, plafonnée à 256 lignes (`pageSize + 1` sert au lookahead), un curseur de continuation, et une promotion atomique par ligne. `RecoverLFSPendingMetaObjectsPage` vérifie que le commit candidat est contenu dans la ref cible exacte, met en cache ce contrôle par opération dans la page, puis promeut chaque association indépendamment. Une association dont la ref n’a pas atteint le candidat reste pending; une erreur laisse le curseur avant la ligne en échec. Le récupérateur non paginé demeure présent et inchangé.

Fichiers/symboles scratch consultés : `models/git/lfs.go::GetLFSPendingMetaObjectsByRepositoryPage`, `PromoteLFSPendingMetaObjectByID`; `services/pull/lfs_recovery.go::RecoverLFSPendingMetaObjectsPage`; tests correspondants dans `models/git/lfs_test.go` et `services/pull/lfs_recovery_test.go`. Les hashes exacts sont consignés séparément; aucun de ces fichiers source n’est livré dans ce dossier de preuve.

## Résultats

- Le test de page ajoute quatre associations, avec une ref cible inaccessible et trois accessibles. Une page de budget 2 scanne exactement deux lignes, promeut une, diffère l’autre et annonce une suite; l’appel suivant reprend au curseur transmis et promeut les deux dernières. L’association inaccessible reste pending. Les limites de page 0 et supérieure à 256 sont refusées.
- Le test d’une cible inexistante reçoit une erreur, garde `NextCursorID` avant la ligne et laisse l’association pending.
- Le test de modèle lance 16 goroutines sur la même promotion par ID avec une base SQLite fichier : une seule consomme la ligne, une association active apparaît et aucune ligne pending ne reste. Le test ciblé passe aussi 10 fois sous `-race`.
- Les paquets complets `./services/pull` et `./models/git` passent avec le tag SQLite, sans échec. Journaux bruts : `green-final.log`, `services-pull.log`, `models-git-final.log`, `promote-by-id-race.log` et `promote-by-id-race-repeat.log`. `red.log` conserve l’échec TDD initial attendu : le nouveau symbole n’existait pas avant l’implémentation.

Commandes exécutées avec `GOTOOLCHAIN=local GOPROXY=off` et `/tmp/forgejo-v17-toolchain/go/bin/go` :

```sh
go test -count=1 -v -timeout=20m -tags sqlite ./services/pull \
  -run '^TestRecoverLFSPendingMetaObjects(AfterInterruptedPostReceive|DefersMissingCandidate|DoesNotPromoteFromUnrelatedRef|PageIsBoundedAndResumable)$'
go test -count=1 -timeout=20m -tags sqlite ./services/pull
go test -count=1 -timeout=20m -tags sqlite ./models/git
go test -race -count=1 -timeout=20m -tags sqlite ./models/git \
  -run '^TestConcurrentLFSPendingPromotionByIDIsIdempotent$'
go test -race -count=10 -timeout=20m -tags sqlite ./models/git \
  -run '^TestConcurrentLFSPendingPromotionByIDIsIdempotent$'
```

`gofmt -d` sur les quatre fichiers source est vide. `git diff --check` passe pour les deux fichiers modèles suivis; cette commande ne vérifie pas les fichiers service non suivis du scratch. Détails dans [verification-checks.txt](verification-checks.txt); versions dans [tool-versions.txt](tool-versions.txt).

## Limites et décision

Le curseur n’est pas stocké durablement. Le test passe le curseur d’un appel au suivant dans le même processus; il ne prouve donc pas une reprise après SIGKILL, ne fournit aucun ordonnanceur et ne garantit aucune équité entre dépôts. Quand `HasMore` est faux, un appelant doit réinitialiser le curseur pour refaire un balayage et revoir les lignes différées. Cette politique n’est pas implémentée.

La cohérence des métadonnées est validée seulement à l’intérieur d’une page. Des associations du même `OperationKey` peuvent se trouver sur deux pages; une divergence entre leurs ref/candidat pourrait donc échapper à cette vérification. Les lectures Git et les transactions DB ne forment pas une transaction atomique : une ref peut avancer ou reculer entre le contrôle et la promotion. Le test ne couvre que SQLite et une base fichier locale, pas PostgreSQL, MySQL, plusieurs instances, les autres writers/routes, le stockage distant, la rétention, le nettoyage ou un chemin de production. La méthode paginée n’a aucun appelant de production recensé.

Ce résultat justifie de poursuivre l’étude d’une récupération gouvernée, mais ne justifie ni fork ni adoption du patch. Le prochain travail scratch doit rendre l’état d’opération cohérent à travers les pages, définir un checkpoint/cycle durable et reprenable, puis fixer les règles de rétention/nettoyage avant d’envisager un worker. Le code reste dans le scratch.

Aucun modèle LLM/AgentOS, Forgejo de production ou service externe n’a été utilisé. Aucune mesure comparative ni gain de valeur produit n’est établi.
