# POC-028 — allocations LFS concurrentes avec la configuration SQLite Forgejo

Date : 24 octobre 2026  
Scratch Forgejo : /tmp/forgejo-v17-base-cas-poc  
Révision upstream : v17.0.0, 0d791421f85ca13a63b665053d81ea765fbbc571  
Go : go1.27.3 linux/arm64  
Test : models/git/lfs_generation_mapping_poc_test.go, sous le build tag forgejo_poc_lfs_generation_mapping  
Source et configuration exactes : voir source-hashes.txt, sqlite-config-excerpt.txt et la copie du test jointe.

## Question

Le helper scratch de réservation, exécuté dans le moteur de test Forgejo réellement configuré, attribue-t-il des générations distinctes quand douze opérations distinctes réservent simultanément le même OID?

## Configuration et trajet testé

Le test utilise unittest.PrepareTestDatabase, puis appelle pocReserveGeneration, qui traverse models/db.WithTx et la table générationnelle réservée au POC. Le moteur de test Forgejo construit sa base SQLite partagée en mémoire avec _txlock=immediate (models/unittest/testdb.go::CreateTestEngine); le constructeur de connexion configuré de production ajoute également _busy_timeout=Database.Timeout et _txlock=immediate (modules/setting/database.go::dbConnStrWithHost). L’extrait et les empreintes des deux sources sont joints.

Douze goroutines attendent une barrière, puis demandent des réservations avec le même OID et douze OperationID distincts. Le test vérifie qu’elles réussissent toutes, qu’aucune génération n’est réutilisée, que les générations 1 à 12 existent et que la table conserve douze lignes.

## Résultat

Le test répété dix fois passe 10/10, soit 120 réservations distinctes sans collision. Le paquet models/git, filtré sur ses deux tests POC sous le même build tag, passe également : 2/2, 0.241s. gofmt -d et git diff --check ne signalent pas d’écart.

Commande répétée :

    GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_generation_mapping' ./models/git -run '^TestPOCConcurrentGenerationReservationsUseConfiguredSQLiteTransactions$' -count=10 -v

Commande ciblée du paquet POC :

    GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_generation_mapping' ./models/git -run '^TestPOC' -count=1 -v

## Interprétation et limites

Cette preuve établit seulement que le helper de test pocReserveGeneration, qui enveloppe scan + insertion dans models/db.WithTx, attribue des générations distinctes avec le mode SQLite immédiat du harnais Forgejo. Le helper, le schéma et le test sont expérimentaux; aucun chemin de production LFS n’appelle ces tables. Ce n’est ni un test d’upload Git LFS réel, ni une migration ou une modification de Forgejo.

Les douze opérations sont concurrentes dans un seul processus et partagent la DB de test en mémoire; aucun worker externe, processus indépendant ou fichier SQLite multi-processus n’est testé. Les autres dialectes et backends restent sans preuve. Le test n’exerce pas l’idempotence de répétition du même OperationID, le stage objet, une panne, la promotion ou les delete/GC.

Cette mesure corrige la portée du POC-027 : son autre fixture utilise xorm.NewEngine avec le chemin SQLite brut et sans les paramètres _txlock=immediate / _busy_timeout de Forgejo. Son database is locked décrit le comportement du modèle différé et ne doit pas être imputé à la base Forgejo configurée. POC-028 observe le chemin de réservation transactionnel test-only avec la configuration du harnais, et non les writers de production.

La suite utile est un scénario de processus séparés contre un fichier SQLite avec les mêmes options, incluant arrêt/reprise et répétition d’opération, puis un audit du raccordement complet aux writers avant tout changement upstream ou produit.
