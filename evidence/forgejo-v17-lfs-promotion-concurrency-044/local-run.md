# Forgejo v17 — promotions LFS concurrentes (POC 044)

Date : 25 octobre 2026. Scratch officiel Forgejo v17.0.0, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571` (GPL-3.0-or-later). Le checkout `/tmp/forgejo-v17-base-cas-poc` est fortement modifié par les POC précédents. Aucun code du scratch n’est intégré au dépôt produit, publié ou déployé; aucune dépendance n’a été ajoutée.

## Question vérifiée

`models/git/lfs.go::PromoteLFSPendingMetaObjects` rend-il une même opération pending active et rejouable lorsque plusieurs appels arrivent simultanément ? Le test `models/git/lfs_test.go::TestConcurrentLFSPendingPromotionIsIdempotent` démarre 16 goroutines pour le même `OperationKey`, attend les appels, puis vérifie l’absence d’erreur, une association active de la bonne taille et zéro ligne pending.

Le premier essai a utilisé le harnais SQLite mémoire partagé de Forgejo et a observé `database table is locked`. La sortie brute de cet essai exploratoire n’a pas été archivée; il ne constitue donc pas une preuve reproductible d’un défaut de Forgejo. Le test a été isolé dans un fichier SQLite temporaire avec le mode de transaction immédiat et un délai d’attente explicite, comme le DSN utilisé par Forgejo; le délai du test est limité à 500 ms. Le DSN applicatif utilise `_busy_timeout` et `_txlock=immediate` (`modules/setting/database.go`, valeur par défaut `SQLITE_TIMEOUT=60000`).

## Résultats observés

Toolchain : `go1.27.3 linux/arm64`; réseau désactivé (`GOPROXY=off`), dépendances en lecture seule (`-mod=readonly`).

- Le test ciblé avec `-count=1 -v` passe : `TestConcurrentLFSPendingPromotionIsIdempotent` (0,06 s; paquet 0,469 s).
- Le même test avec `-count=25` passe : `ok forgejo.org/models/git 2.177s`.
- Le test avec le détecteur de courses (`go test -race`, `-count=1`) passe : `ok forgejo.org/models/git 1.891s`.
- Le paquet complet `./models/git -count=1` passe : `ok forgejo.org/models/git 0.763s`.

Les commandes complètes et leurs résumés de sortie sont dans [test-results.txt](test-results.txt). Les empreintes SHA-256 des sources pertinentes sont dans [scratch-source-sha256.txt](scratch-source-sha256.txt).

## Portée et suite

Ceci est un test de modèle dans un processus avec un pool SQLite fichier partagé, pas une simulation multi-processus ou plusieurs nœuds Forgejo. Il ne teste ni récupération lancée au démarrage, ni interruption réelle entre mise à jour du ref Git et promotion, ni backend PostgreSQL/MySQL, ni stockage ou client LFS. Il ne câble aucun appelant à `RecoverLFSPendingMetaObjects`; l’audit de 043 n’en a toujours trouvé aucun hors définition et tests. La preuve ne dit pas non plus que tous les chemins d’écriture Forgejo appliquent la précondition de base exigée.

Prochaine étape scratch : concevoir un claim/lease de récupération qui fonctionne entre processus ou nœuds et reste reprenable après expiration, puis faire un test fichier-backed multi-processus et vérifier le chemin après interruption. Avant intégration d’un worker, définir rétention, nettoyage, observabilité et comportement d’échec; ne pas ajouter un cron par processus en s’appuyant sur le verrou cron local. Garder séparée l’analyse des writers et de la base éprouvée pour API/Web/AutoMerge/rebase. Le POC n’est pas une recommandation de fork.

## Manifestes

Le manifeste local couvre ce rapport, les résultats et les empreintes; il ne couvre pas lui-même. Le manifeste global produit contient 2 994 lignes; en prenant la dernière empreinte pour chaque chemin répété, ses 2 992 chemins présents sont concordants, sans chemin manquant. Le seul placeholder historique malformé (`None  tests/test_mission_012_live_composed_e2e.py`) est conservé. Les empreintes du dépôt produit ne remplacent pas les hashes de sources ci-dessus.
