# POC-057 — checkpoint durable et cycles LFS bornés dans le scratch Forgejo v17

Date : 25 octobre 2026. Scratch : `/tmp/forgejo-v17-base-cas-poc`, dépôt officiel Forgejo `v17.0.0`, HEAD de base `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch avait déjà 51 chemins modifiés/non suivis avant ce lot; ils ont été conservés. Aucun code Forgejo n’a été copié au dépôt produit.

## Lacune et mécanisme éprouvé

POC-055/056 fournissait une lecture keyset bornée et l’identité immuable des associations, mais aucun état de balayage durable. Le nouveau modèle expérimental `LFSPendingMetaRecoveryCheckpoint` stocke par dépôt le curseur, une borne haute et la génération du balayage. Le début d’un cycle capture l’ID pending maximal; chaque page lit seulement `cursor < id <= high_water`, et traite au plus le budget demandé (limite existante : 256 associations). La progression s’enregistre par CAS sur dépôt, curseur, borne et génération.

Après la dernière page, le curseur et la borne sont remis à zéro et la génération est incrémentée. Le prochain appel capture un nouveau maximum et repart du début : une association différée est donc réexaminée, tandis qu’une arrivée postérieure à la borne ne rallonge pas indéfiniment le cycle en cours. Si la récupération échoue avant le CAS, l’ancien checkpoint reste; les promotions déjà validées sont rejouables, car une association déjà consommée devient un no-op. Un CAS obsolète est refusé au lieu d’écraser l’état d’une génération plus récente.

Le test d’intégration `TestRecoverLFSPendingMetaObjectsCheckpointSurvivesReopenAndRevisitsDeferredRows` utilise SQLite fichier et un dépôt Git nu réel. Avec une page de taille 1, il laisse d’abord une association différée, persiste son curseur, ajoute une nouvelle association après la borne, simule l’interruption après la transaction de promotion et avant le CAS, ferme et rouvre SQLite, puis reprend sans fournir de curseur en mémoire. Il vérifie que le balayage borné se termine, n’effectue pas une seconde promotion, remet à zéro le checkpoint et revisite ensuite l’association différée au cycle suivant. Une tentative d’écriture à partir d’un snapshot CAS périmé ne modifie pas le checkpoint récent.

## TDD et résultats

Le test a d’abord échoué à la compilation avec les symboles checkpoint absents (`red.log`), avant toute implémentation. Après ajout du modèle et du service, le scénario ciblé passe. La sortie finale ciblée et celle du détecteur de races sont dans `targeted-test-final.log` et `race-test.log`.

Vérifications finales :

```text
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags sqlite ./services/pull -run '^TestRecoverLFSPendingMetaObjectsCheckpointSurvivesReopenAndRevisitsDeferredRows$' -count=1 -v -timeout=10m
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -race -tags sqlite ./services/pull -run '^TestRecoverLFSPendingMetaObjectsCheckpointSurvivesReopenAndRevisitsDeferredRows$' -count=1 -timeout=20m
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -p 1 -tags sqlite ./models/forgejo_migrations ./models/git ./services/pull -count=1 -timeout=20m
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags sqlite ./models/git -run '^TestConcurrentLFSPendingPromotionByIDIsIdempotent$' -count=10 -timeout=10m
/tmp/forgejo-v17-toolchain/go/bin/gofmt -d models/git/lfs.go services/pull/lfs_recovery.go services/pull/lfs_recovery_test.go
git diff --check
```

Le scénario ciblé passe; le test de reprise passe aussi sous `-race`. Les trois paquets SQLite (`models/forgejo_migrations`, `models/git`, `services/pull`) passent ensemble avec `-p 1`. Le test existant de promotion concurrente passe isolément puis 10/10 répétitions. `gofmt -d` est vide, `git diff --check` passe et le manifeste local vérifie 16 artefacts (`SHA256SUMS`), résultat conservé dans `local-manifest-check.txt`. Versions, empreintes de sources et sortie finale sont dans les fichiers voisins.

Deux exécutions initiales non vertes sont préservées : `package-tests.log` a révélé que le nouveau test avait sauvegardé une session au lieu de l’engine global pour la restauration; le test utilise maintenant le modèle établi de `lfs_recovery_multiprocess_poc_test.go` et les paquets complets passent en série. `package-tests-final.log` a ensuite échoué une fois sur `TestConcurrentLFSPendingPromotionByIDIsIdempotent` avec `database is locked`; un run isolé et 10 répétitions passent, de même que la suite sérielle. La cause de cette contention unique n’est pas établie et elle n’est pas présentée comme résolue.

## Portée et limites

- C’est un prototype de modèle/service et un test de fermeture/réouverture SQLite; aucun serveur Forgejo n’a démarré, aucun SIGKILL de processus n’a été injecté et aucun worker/cron n’est branché.
- Le test reproduit un effet durable avant l’avancement du checkpoint; il ne mesure pas tous les points d’arrêt ni plusieurs processus travaillant simultanément sur le checkpoint.
- Éprouvé seulement sous SQLite sur un hôte; PostgreSQL/MySQL et plusieurs hôtes ne sont pas couverts. La mise à jour du curseur est un CAS, mais il n’y a ni claim/lease distribué ni ordonnanceur global de dépôts.
- La fairness démontrée est bornée au snapshot des pending d’un même dépôt : les nouvelles associations attendent le cycle suivant et les différées sont revisitées. L’équité entre dépôts n’est pas implémentée.
- Les promotions par ligne et le CAS du curseur restent deux transactions distinctes; l’ordre permet une reprise idempotente, mais le test n’injecte pas une panne au milieu d’un commit DB ni un changement concurrent des refs Git.
- Le modèle checkpoint est enregistré pour la synchronisation générique et testé avec `Sync2`; le cycle de démarrage réel et une migration serveur complète ne sont pas testés.
- Tout le code reste dans le scratch non propre; aucun patch n’est adopté, publié ou copié au produit. Aucun LLM, modèle, secret ou service externe n’est utilisé.

Prochaine action : tester l’ordre réel de démarrage/synchronisation puis une reprise par processus indépendant après arrêt, examiner les changements de refs pendant la promotion, et décider si un ordonnanceur équitable inter-dépôts est nécessaire avant tout worker. La mission produit reste incomplète.
