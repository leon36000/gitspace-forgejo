# POC-030 — reprise LFS interprocessus avec transactions SQLite configurées

- Date : 24 octobre 2026
- Source Forgejo auditée : `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`
- Scratch : `/tmp/forgejo-v17-base-cas-poc`
- Go : `go1.27.3 linux/arm64`
- XORM : `code.forgejo.org/xorm/xorm v1.4.1`
- SQLite driver : `github.com/mattn/go-sqlite3 v1.14.52`

## Question et méthode

POC-026 avait démontré le stage, l’arrêt contrôlé puis la récupération avec des moteurs SQLite distincts. POC-029 avait démontré la réservation multiprocessus avec le DSN SQLite configuré de Forgejo (`_busy_timeout=500`, `_txlock=immediate`), mais séparément. Ce test scratch compose les deux avec `models/db.WithTx`, la table de mapping expérimentale et le véritable `storage.LocalStorage`.

Le seul nouveau fichier du scratch pour ce POC est `models/git/lfs_generation_process_recovery_poc_test.go`, sous le build tag `forgejo_poc_lfs_generation_mapping`. Le processus de stage réserve une génération avec `pocReserveGeneration`, écrit la clé immuable au moyen de `LocalStorage.Save`, puis s’arrête par `os.Exit(73)` avant toute promotion. Le parent rouvre la base et le stockage et constate une ligne `pending`, des octets présents et aucune association courante. Un processus distinct appelle `pocRecoverGeneration`, qui vérifie taille, OID et SHA-256 puis promeut sous `db.WithTx`. Un troisième processus rejoue la récupération; le mapping reste identique et l’objet actif se relit avec les octets attendus.

Le scénario ciblé passe cinq fois. La suite des six tests scratch `TestPOC*` de `models/git`, incluant les tests précédents de réservation, contention, retry et reprise, passe également. Les journaux et la copie exacte du test sont dans ce dossier. Les hashes du nouveau test et des points d’intégration Forgejo consultés sont dans `source-hashes.txt`.

## Commandes et résultats

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_generation_mapping' \
  ./models/git \
  -run '^TestPOCConfiguredSQLiteStageRecoversAndPromotesAcrossProcesses$' \
  -count=5 -v
```

Résultat : 5/5 parcours interprocessus réussis.

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_generation_mapping' \
  ./models/git -run '^TestPOC' -count=1 -v
```

Résultat : les six tests scratch sélectionnés passent. Le test parent de single-attempt attend le refus `database is locked`; son sous-processus apparaît comme échec attendu dans le journal, tandis que le test parent confirme l’absence de réservation partielle.

Le premier lancement omettait les build tags SQLite et s’arrêtait au `TestMain` avant d’exécuter le scénario. Après ajout des tags, les premiers enfants n’avaient pas `HOME` puis `PATH`, nécessaires à l’initialisation du harnais Forgejo. Le test transmet maintenant uniquement ces variables d’environnement de harnais ainsi que `PROJECT_ROOT`, `PROJECT_CONF`, `GIT_CONFIG_NOSYSTEM` et `TMPDIR`; les deux commandes ci-dessus ont ensuite réussi. Ces erreurs concernaient le lanceur de test, pas le stockage ni une route produit.

## Portée et limites

La preuve couvre un seul hôte Linux, une base SQLite fichier avec les paramètres du harnais Forgejo, une clé d’objet stockée par `LocalStorage`, une sortie de processus après l’achèvement de `Save`, puis la reprise dans un processus différent. Elle ne couvre pas une coupure pendant l’écriture, une panne matérielle, plusieurs hôtes, PostgreSQL/MySQL, NFS ou un backend objet distant.

Les tables et helpers `pocLFSGeneration`, `pocLFSCurrentMapping`, `pocReserveGeneration` et `pocRecoverGeneration` sont expérimentaux. Aucun writer LFS Forgejo, route HTTP, hook, merge, migration, URL, suppression, Doctor, GC ou mécanisme de rétention n’est raccordé à ce POC. Il ne valide donc pas l’application d’une précondition à tous les writers et ne justifie aucun fork. Les modifications préexistantes du scratch Forgejo sont conservées; ce lot n’y ajoute qu’un fichier de test sous build tag. Aucun code de Forgejo n’a été copié au dépôt produit.

## Suite utile

Raccorder un adaptateur de mapping test-only à un writer ou handler LFS réel dans le harnais Forgejo, puis vérifier le readback après reprise et promotion. Fermer séparément la compatibilité des writers directs et les interactions avec suppression, Doctor/GC, URL et `Resolve/Open`. Chiffrer le coût upstream avant toute décision de fork. Le parcours LLM et la comparaison contrôlée avec Forgejo + agent + CI conventionnelle restent non démontrés.
