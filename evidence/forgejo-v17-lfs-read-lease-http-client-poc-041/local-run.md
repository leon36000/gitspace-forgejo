# POC-041 — client HTTP réel et détection d’un stream LFS tronqué

Date : 24 octobre 2026. Le test s’exécute dans le scratch local `/tmp/forgejo-v17-base-cas-poc`, sur Forgejo officiel v17.0.0, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571`, worktree détaché et modifié. Les sources scratch ne correspondent donc pas à un arbre amont propre. Les empreintes ciblées sont dans [`source-sha256.txt`](source-sha256.txt).

## Résultat observé

Le test traverse le routeur d’intégration réel de Forgejo et son `DownloadHandler`, avec le `LFSDelete` réel. Le stockage générationnel, la ligne de lease et le claim GC sont fournis par les adaptateurs de test SQLite des POC-037 à -040; deux moteurs XORM séparés utilisent le même fichier. Le test provoque la suppression logique pendant la lecture, laisse expirer le lease, fait gagner le claim CAS GC, puis débloque le renouvellement qui doit refuser la génération déjà réclamée.

La requête GET passe par un vrai `httptest.Server` et un `http.Client` Go, sans client simulé. La réponse est HTTP 200 avec `ContentLength=65 536`; la lecture remet 16 384 octets, puis `io.ReadAll` retourne `io.ErrUnexpectedEOF`. Le finaliseur de fixture termine la suppression et les readbacks vérifient `generation=deleted`, `lease=released`, objet absent et association LFS absente. L’auto-décompression HTTP est désactivée dans le client de mesure pour conserver la longueur du corps déclarée.

Le scénario passe une fois, deux fois de suite, puis sous le détecteur de courses Go. Les logs sont [`client-test.log`](client-test.log), [`repeat-2.log`](repeat-2.log) et [`race.log`](race.log). Le routeur écrit un message `log.Error` attendu quand il refuse le stream; le test Go passe malgré ce message.

## Itérations du harnais conservées

Les logs [`harness-buffer-red.log`](harness-buffer-red.log), [`harness-content-length-red.log`](harness-content-length-red.log) et [`harness-gate-order-red.log`](harness-gate-order-red.log) conservent trois échecs de mesure, pas des régressions Forgejo : une petite réponse n’était pas encore livrée par le buffer HTTP pendant que la lecture restait bloquée; le transport standard décompressait la réponse et masquait sa longueur de transfert; enfin, attendre la réponse avant de libérer le renouvellement créait un timeout dans le test. Le fixture final utilise un corps plus grand, désactive la décompression et libère l’opération après le claim avant de lire le corps. Les sorties finales répétées et `-race` passent.

## Commandes et environnement

Le détail des outils, variables et tags figure dans [`tool-versions.txt`](tool-versions.txt). Les commandes sont exécutées hors ligne avec Go 1.27.3 linux/arm64 et SQLite :

~~~sh
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off GOOS=linux GOARCH=arm64 CGO_ENABLED=1 \
  /tmp/forgejo-v17-toolchain/go/bin/go test -v -count=1 \
  -tags='sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' \
  ./tests/integration -run '^TestPOCSQLiteRouteRealHTTPClientDetectsLostLeaseAsUnexpectedEOF$'

GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off GOOS=linux GOARCH=arm64 CGO_ENABLED=1 \
  /tmp/forgejo-v17-toolchain/go/bin/go test -v -count=2 \
  -tags='sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' \
  ./tests/integration -run '^TestPOCSQLiteRouteRealHTTPClientDetectsLostLeaseAsUnexpectedEOF$'

GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off GOOS=linux GOARCH=arm64 CGO_ENABLED=1 \
  /tmp/forgejo-v17-toolchain/go/bin/go test -race -v -count=1 \
  -tags='sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_lfs_read_lease_stream' \
  ./tests/integration -run '^TestPOCSQLiteRouteRealHTTPClientDetectsLostLeaseAsUnexpectedEOF$'
~~~

`gofmt -d` et `git diff --check` ne rapportent aucune différence. Le test est lié au tag `forgejo_poc_*` et n’est ni un changement produit, ni un correctif proposé à Forgejo.

## Limites et suite

Cette preuve confirme uniquement que le client détecte une réponse de longueur incomplète après perte du lease. Elle n’établit pas que la déconnexion du client annule un read objet déjà bloqué, que chaque backend respecte `Close`, ni que le renouvellement respecte un budget total. Le claim et le finaliseur GC restent des fixtures, SQLite est le seul moteur testé et LocalStorage le seul backend. Aucun worker GC durable, reprise après panne réelle, S3/MinIO, test multi-processus ou intégration production n’est exercé.

Prochaine tranche ciblée : fermer le client HTTP pendant une lecture storage effectivement bloquée et mesurer si le contexte annule le read; vérifier séparément un délai total borné et un adaptateur `Close` qui interrompt le read. Ensuite seulement, éprouver la reprise du worker GC et les autres backends. Ne pas adopter ni copier les modifications Forgejo sans décision documentée et validation des licences.
