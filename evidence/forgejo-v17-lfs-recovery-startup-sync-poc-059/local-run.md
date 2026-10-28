# POC-059 — synchronisation du modèle checkpoint enregistré

Date : 25 octobre 2026. Scratch : `/tmp/forgejo-v17-base-cas-poc`, dépôt officiel Forgejo `v17.0.0`, commit de base `0d791421f85ca13a63b665053d81ea765fbbc571`. Expérience scratch uniquement; aucun code Forgejo n’a été copié au produit.

## Constat dans le code Forgejo v17

La route de démarrage `routers/common/db.go::InitDBEngine` appelle `db.InitEngineWithMigration` avec `migrateWithSetting`; ce callback appelle le dispatcher `gitea_migrations.Migrate` lorsque l’auto-migration est activée ou que la base n’est pas initialisée (`routers/common/db.go`, lignes 22–56). `models/db/engine.go::InitEngineWithMigration` appelle ensuite `migrateFunc`, puis `SyncAllTables`, puis les fonctions d’initialisation enregistrées (lignes 257–288). `SyncAllTables` trie et synchronise les modèles de la liste globale `tables` (lignes 175–185); `RegisterModel` ajoute un bean à cette liste (lignes 83–88). Enfin `models/git/lfs.go::init` enregistre `LFSPendingMetaRecoveryCheckpoint` avec les autres tables LFS (lignes 147–171).

Cette inspection couvre uniquement ces call sites et l’enregistrement du modèle. Elle ne garantit pas qu’une version de migration ou un chemin d’installation précis ait été exécuté sur une vraie base; le helper de récupération et le dispatcher complet ne sont pas démarrés ici.

## Test effectué

Un nouveau test sous build tag, `services/pull/lfs_recovery_startup_sync_poc_test.go::TestPOCLFSPendingRecoveryCheckpointIsRegisteredForStartupSchemaSync`, ouvre SQLite fichier, configure l’engine global du test, appelle `db.SyncAllTables()` et vérifie avec `IsTableExist` que la table du checkpoint a été créée depuis les modèles enregistrés. Il utilise les dépendances déjà présentes; aucun réseau ni service externe.

Commande finale :

```text
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags sqlite,forgejo_poc_lfs_recovery_multiprocess ./services/pull -run '^TestPOCLFSPendingRecoveryCheckpointIsRegisteredForStartupSchemaSync$' -count=1 -v -timeout=10m
```

Résultat : `PASS`, scénario 1/1 et paquet `services/pull` réussi en 6,456 s. `gofmt -d` et `git diff --check` passent. Le premier essai n’a pas compilé parce que le nouveau test importait `names` depuis `forgejo.org/modules/names`; les tests du scratch utilisent `code.forgejo.org/xorm/xorm/names`. Le chemin d’import a été corrigé puis le test a passé. Les deux sorties sont conservées dans `initial-import-failure.log` et `startup-sync-final.log`; les vérifications, versions et SHA-256 sont dans les fichiers voisins.

## Portée et limites

- Le test passe par `SyncAllTables`, mais ne lance pas `routers/common.InitDBEngine` ni `InitEngineWithMigration`, ne configure pas le moteur de migration complet et ne démarre pas Forgejo. Il confirme le modèle enregistré et la création de sa table par la synchronisation, pas un upgrade bout en bout.
- SQLite seulement, un test, un hôte. Pas de PostgreSQL/MySQL, interruption pendant migration, concurrence de démarrage, worker de reprise, mutation du ref Git entre observation et promotion, ou équité inter-dépôts.
- Le nouveau fichier est un test scratch sous build tag. Aucun changement du produit, aucune adoption de fork, publication, dépense, appel LLM ou donnée privée externe.

Suite proposée : monter un test avec le dispatcher de migrations réellement configuré sur une base isolée, puis construire un point d’injection déterministe pour déplacer le ref entre la lecture Git et la promotion LFS. Avant tout worker, spécifier la fairness inter-dépôts et la rétention/nettoyage. La mission produit reste incomplète.
