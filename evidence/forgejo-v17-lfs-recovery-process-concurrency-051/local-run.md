# Forgejo v17 — récupération LFS interprocessus (POC 051)

Date locale : 25 octobre 2026. Source Forgejo officielle auditée : v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`, licence GPL-3.0-or-later. Le scratch `/tmp/forgejo-v17-base-cas-poc` contient les expériences précédentes non adoptées; ce test est ajouté sous le build tag `forgejo_poc_lfs_recovery_multiprocess`. Le dépôt produit ne reçoit aucun code Forgejo.

## Question

La promotion de métadonnées LFS en attente reste-t-elle correcte si plusieurs processus de récupération Forgejo ouvrent indépendamment le même SQLite fichier-backed et appellent `services/pull/lfs_recovery.go::RecoverLFSPendingMetaObjects` en même temps ? Le test emploie les modèles réels `LFSMetaObject`/`LFSPendingMetaObject` et le helper de promotion `models/git/lfs.go::PromoteLFSPendingMetaObjects`; il ne crée pas de schéma de génération parallèle.

## Méthode et résultats

Un seul processus de test prépare une ligne pending et un dépôt Git nu avec le commit candidat à `refs/heads/main`. Quatre processus enfants du binaire de test ouvrent leurs propres moteurs XORM SQLite vers ce fichier, attendent une barrière de départ commune puis exécutent le vrai helper de récupération. Le DSN de test reprend les options configurées de Forgejo : `_busy_timeout=500`, `_txlock=immediate`, `mode=rwc`. Le processus parent relit ensuite le modèle actif et le nombre de lignes pending dans la base.

- Répétition ciblée : 10/10 exécutions passent; chaque exécution lance quatre processus récupérateurs. Aucun enfant ne rapporte d’erreur ni d’opération différée. La relecture confirme l’association active de taille 123 et zéro association pending.
- Détecteur Go `-race` : 1/1 exécution passe.
- Tout le paquet `./services/pull`, avec ce build tag : passe (`ok forgejo.org/services/pull 1.585s`).
- Versions : Go `go1.27.3 linux/arm64`, XORM `v1.4.1`, driver SQLite `v1.14.52`. Le commit officiel et les empreintes des sources du scratch sont conservés dans les fichiers joints.

Commandes reproductibles depuis le scratch, réseau Go coupé :

```sh
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_recovery_multiprocess' \
  ./services/pull \
  -run '^TestPOCLFSPendingRecoveryIsIdempotentAcrossProcesses$' \
  -count=10 -v

GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -race \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_recovery_multiprocess' \
  ./services/pull \
  -run '^TestPOCLFSPendingRecoveryIsIdempotentAcrossProcesses$' \
  -count=1 -v

GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_recovery_multiprocess' \
  ./services/pull -count=1
```

Les sorties sont dans `concurrency-repeat-10.log`, `race.log` et `services-pull-all.log`. La copie exacte du test scratch, le HEAD officiel, l’état des chemins concernés, les versions et les hashes sont également présents ici.

## Interprétation et limites

Ces observations montrent que les transactions et l’idempotence actuelles tolèrent des appels concurrents entre quatre processus sur un hôte Linux avec SQLite fichier-backed. Pour la **correction de la promotion** dans ce cas précis, le test ne fournit donc pas de preuve qu’un lease séparé est nécessaire. Cela n’établit ni un ordonnancement efficace ni une reprise automatique au démarrage : l’audit du scratch n’a toujours trouvé aucun appelant de production du helper `RecoverLFSPendingMetaObjects`.

Le scénario ne démarre pas Forgejo; il n’exerce ni route HTTP, ni upload d’octets LFS, ni interruption pendant la transaction, ni plusieurs hôtes, ni PostgreSQL/MySQL, NFS ou backend objet distant. Il ne définit pas le scan borné, la planification, les retries après panne, la rétention, le nettoyage ou l’observabilité. Les objets LFS de ce test sont des métadonnées synthétiques; seule l’association active est vérifiée. Les résultats ne justifient donc ni un worker de démarrage par instance, ni un fork.

## Point d’appel et limites d’exploitation constatés dans le code v17

L’examen ciblé donne les points suivants, tous au commit officiel listé plus haut : `routers/init.go` appelle `cron.NewContext` à la fin de l’initialisation serveur; `services/cron/cron.go::NewContext` crée un scheduler avec `gocron.NewScheduler()` dans chaque processus. `services/cron/tasks.go::Task.RunWithUser` ne bloque les doublons que via le `taskStatusTable` mémoire local au package/processus. `services/cron/tasks_extended.go::registerGCLFS` enregistre le GC LFS, désactivé par défaut et sans exécution au démarrage, avec périodicité de 24 h lorsque configuré.

Le GC ne peut pas être réutilisé tel quel pour la reprise : `services/repository/lfs.go::GarbageCollectLFSMetaObjectsForRepo` itère les `LFSMetaObject` actifs; le helper de reprise séparé charge tous les pending d’un dépôt via `models/git/lfs.go::GetLFSPendingMetaObjectsByRepository` sans pagination avant de les examiner. Le helper laisse les opérations non résolues en attente et aucun appel de production au helper n’a été recensé. Cela implique, par lecture du code, qu’ajouter simplement un cron ici ferait exécuter le scan dans chaque instance active; les promotions identiques tolèrent la concurrence testée, mais leur coût de scan/commandes Git n’est pas coordonné ni borné. Les sources de ce constat figurent dans `scratch-source-sha256.txt`.

Suite : avant tout wiring, définir une sélection paginée et une borne de travail par passage, ainsi que la politique de retry, rétention et nettoyage des pending. Décider ensuite si une coordination DB réduit un coût mesuré; le test 051 ne la rend pas nécessaire à l’intégrité de l’association sous le cas SQLite testé. Tester plusieurs instances, une erreur DB/scan et une interruption/reprise avant toute intégration. Continuer séparément l’audit du transport de la base attestée vers les writers. Conserver ce patch dans le scratch et ne pas estimer un coût de rebase sans comparaison upstream dédiée.
