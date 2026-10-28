# POC-027 — reprises LFS concurrentes sur SQLite

Date : 24 octobre 2026  
Scratch Forgejo officiel : /tmp/forgejo-v17-base-cas-poc  
Révision de base : v17.0.0, 0d791421f85ca13a63b665053d81ea765fbbc571  
Go : go1.27.3 linux/arm64  
Test tagué : services/lfs/versioned_download_sqlite_poc_test.go  
Empreinte du test exécuté : indiquée dans scratch-source-sha256.txt.

## Question et méthode

Deux appels de reprise portant la même opération durable peuvent-ils promouvoir sans corrompre l’unique mapping courant quand ils utilisent deux moteurs XORM distincts vers le même fichier SQLite?

Le test réserve un stage vérifié dans SQLite + LocalStorage, puis lance deux récupérations en parallèle via deux instances XORM. Une barrière test-only les arrête après lecture du mapping courant, avant l’écriture; elle force donc les deux transactions à observer le même état initial. Après les deux résultats, les erreurs de contention sont retentées sur le même moteur, puis les connexions sont fermées et rouvertes pour une seconde reprise. Le test relit enfin le mapping et l’état de l’opération.

## Résultat

Le test répété dix fois passe 10/10. Dans chacune des dix répétitions, une promotion retourne database is locked et l’autre finit; après achèvement des deux tentatives, le retry de l’opération échouée passe dans les dix cas. Fermer puis rouvrir les moteurs, relancer la récupération et la rejouer donne aussi le mapping actif attendu dans les dix cas.

Commande répétée :

    GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_download' ./services/lfs -run '^TestPOCConcurrentLFSRecoveryAcrossSQLiteEnginesRecoversAfterRestart$' -count=10 -v

Le paquet complet services/lfs sous ce build tag passe également avec les trois tests POC présents : 0 échec, 0.333s. gofmt -d et git diff --check sont propres. Les logs, la source exacte, l’état scratch et les empreintes sont conservés dans ce dossier.

## Cause et portée

Dans cette fixture seulement, deux transactions SQLite ont lu le mapping absent avant d’écrire; la promotion avec Begin différé a rendu database is locked dans un cas concurrent, et le retry après la fin des deux appels a convergé. Le test répétitif passe 10/10.

La fixture a été faite avec xorm.NewEngine("sqlite3", chemin) sans paramètres de connexion Forgejo. L’essai force deux lectures avant promotion, puis vérifie le retry après la fin des deux transactions. Forgejo configure SQLite différemment : le harnais unitaire emploie _txlock=immediate et le constructeur de connexion ajoute aussi _busy_timeout. Ce diagnostic ne doit donc pas être attribué à la configuration Forgejo.

Limites : tables et adaptateur de test uniquement, LocalStorage, deux connexions XORM au sein d’un seul processus. Aucun writer Forgejo de production, route, allocation concurrente, processus concurrent distinct, PostgreSQL/MySQL, S3/MinIO, suppression conditionnelle, GC, migration ou DB à haute charge n’est testé. Le test ne démontre ni le protocole complet ni la sûreté multi-processus. Aucune source Forgejo de production ou produit n’est modifiée par ce lot et aucun fork n’est adopté. Le DSN scratch n’inclut ni _txlock=immediate ni _busy_timeout.

La suite doit couvrir les collisions d’allocation d’opérations distinctes et les reprises multi-processus, puis les suppressions anciennes après promotion. Il faut choisir une stratégie transactionnelle et un budget de retry par dialecte avant tout câblage; le handler et l’inventaire Doctor/GC restent hors de ce POC.
