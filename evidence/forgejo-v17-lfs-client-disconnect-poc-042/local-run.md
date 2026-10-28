# POC-042 — déconnexion HTTP pendant une lecture LFS bloquée

Date : 24 octobre 2026. Essai isolé dans `/tmp/forgejo-v17-base-cas-poc`, Forgejo officiel v17.0.0, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch contient déjà des adaptations locales des POC LFS précédents; les sources concernées et leurs empreintes sont référencées dans [`source-sha256.txt`](source-sha256.txt). Aucun changement n’a été copié dans le dépôt produit ni soumis upstream.

## Résultat

Le test passe par le routeur LFS d’intégration réel, un serveur `httptest.Server` et un client `net/http` réel. Après réception d’octets, le client annule sa requête et ferme son corps pendant que l’objet de stockage de fixture bloque sa seconde lecture. Le test capte le contexte reçu par `OpenLFSGeneration`, constate son annulation, puis observe `Close` sur l’objet, la fin du handler, le lease en état `released` et la génération retirée supprimée.

Le test rouge, avec le changement expérimental absent, échoue de façon actionnable : le contexte serveur est annulé mais l’objet dont `Read` est bloqué ne reçoit pas `Close` avant l’échéance de 2 s. Dans le scratch seulement, `renewingLFSReadObject.run` appelle désormais `closeObject()` lorsque son contexte se termine. Le test ciblé passe ensuite une fois et sous `-race`; aucun rapport de course n’apparaît. Les sorties sont dans [`red.log`](red.log), [`client-disconnect.log`](client-disconnect.log) et [`race.log`](race.log).

## Commandes et environnement

Go `1.27.3`, `linux/arm64`; SQLite et `LocalStorage`. Commande ciblée, exécutée avec `GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off GOOS=linux GOARCH=arm64 CGO_ENABLED=1` :

~~~sh
/tmp/forgejo-v17-toolchain/go/bin/go test -v -count=1 \
  -tags='sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' \
  ./tests/integration -run '^TestPOCSQLiteRouteClientDisconnectClosesBlockedRead$'

/tmp/forgejo-v17-toolchain/go/bin/go test -race -v -count=1 \
  -tags='sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' \
  ./tests/integration -run '^TestPOCSQLiteRouteClientDisconnectClosesBlockedRead$'
~~~

`gofmt -d` sur les deux fichiers scratch et `git diff --check` ne produisent aucune sortie; résultats enregistrés dans [`gofmt-check.txt`](gofmt-check.txt) et [`diff-check.txt`](diff-check.txt). Le logger d’intégration affiche `FATAL ERROR` pour des `log.Error` attendus (lecture interrompue et tâches annulées); ce texte n’est pas le verdict Go. Les logs verts terminent par `PASS`.

## Limites et suite

La lecture bloquée est contrôlée par `pocSQLiteRenewableRouteObject`, un adaptateur de test qui se débloque quand `Close` est appelé. Le test ne prouve pas que tous les backends honorent cette convention : S3/MinIO, réseau interrompu, annulation d’un read distant, débit de renouvellement maximal, délai total de lease, worker GC durable et reprise après crash ne sont pas testés. Le lease, le claim et le readback de génération restent des fixtures SQLite; aucun worker GC Forgejo n’est exécuté.

Cette modification est une piste scratch, pas un patch Forgejo adopté. Son comportement dépend de la sémantique concurrente `Close`/`Read` de chaque backend; cette compatibilité et son coût de maintenance upstream restent à examiner. Prochaine tranche : rendre le renouvellement borné sur la durée totale, puis exercer le claim/finaliseur GC à travers une panne et une reprise idempotente, sans prétendre que les fixtures démontrent un worker de production.
