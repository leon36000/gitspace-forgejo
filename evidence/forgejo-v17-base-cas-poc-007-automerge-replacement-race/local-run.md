# FORGEJO-BASE-CAS-POC-007 — génération de schedule AutoMerge (24 octobre 2026)

## Base et question vérifiée

- Forgejo officiel v17.0.0, base `0d791421f85ca13a63b665053d81ea765fbbc571`; expérience scratch cumulative `/tmp/forgejo-v17-base-cas-poc`, sans commit de travail et sans reprise de code dans le produit.
- Toolchain Go `1.27.3 linux/arm64`, SQLite, cache/temp sous `/dev/shm`, `GOMAXPROCS=4`.
- `TestPullMergeStaleAutoMergeWorkerCannotConsumeReplacementSchedule` planifie A, suspend le worker après lecture de A, annule A par API (`204`), programme B avec un autre style de fusion et un nouvel ID, puis reprend le worker.
- Les empreintes SHA-256 des trois fichiers scratch concernés figurent dans `scratch-source-sha256.txt`.

## Rouge et changement scratch

Le test rouge montre que l’ancien worker supprimait B par le seul ID de PR, puis fusionnait avec les paramètres mémorisés d’A. L’assertion constate la disparition du schedule B après la fusion.

Le modèle ajoute `DeleteScheduledAutoMergeWithID(ctx, pullID, scheduleID)`, qui supprime seulement la ligne dont l’ID et le PR ID correspondent et renvoie `ErrNotExist` quand cette génération n’existe plus. Le worker passe l’ID initialement lu; une annulation ou un remplacement antérieur fait donc abandonner ce worker. Le test vert vérifie que B, son nouvel ID et son style Squash restent présents, que la PR reste ouverte et que la branche ne bouge pas.

## Vérifications

- Test rouge : `automerge-replacement-red.log` — le worker consomme B et fusionne avec A.
- Test vert : `automerge-replacement-green.log` — B reste intact après reprise du worker ancien.
- Régressions : `automerge-regressions-final.log` — 8 fonctions d’intégration (AutoMerge après statuts/approbation, fork, retrait, transition et courses POC-005/006/007) passent; paquet `forgejo.org/tests/integration`, `24.272s`.
- Services : `service-tests.log` — `./services/automerge` et `./services/pull` passent (`0.221s`, `0.374s`). `git diff --check` passe.

Commande des régressions finales :

```sh
GOCACHE=/dev/shm/codex-forgejo-v17-go-cache TMPDIR=/dev/shm GOMAXPROCS=4 \
  /tmp/forgejo-merge011-test/toolchain/bin/go test -count=1 -v -timeout=20m -tags sqlite \
  ./tests/integration -run '^(TestPullAutoMergeAfterCommitStatusSucceed|TestPullAutoMergeAfterCommitStatusSucceedAndApprovalForAgitFlow|TestPullRemoveAutomerge|TestPullAutoMergeFromFork|TestPullMergeRepositoryPolicyPreservesPreviouslyScheduledAutoMergeForCancellation|TestPullMergeRepositoryPolicyRacePreservesScheduledAutoMerge|TestPullMergeAutoMergeCancellationBeforeScheduleClaimWins|TestPullMergeStaleAutoMergeWorkerCannotConsumeReplacementSchedule)$'
```

## Revue indépendante et limite restante

La revue lecture seule confirme que la suppression `(scheduleID, pullID)` ferme la consommation de B testée. Elle relève toutefois une autre course P2 : si A a déjà été pris en charge/supprimé, qu’un utilisateur crée puis annule B (`204`), et que la politique stricte fait ensuite refuser la fusion d’A, le chemin de restauration peut recréer A. Le code ne conserve pas de génération durable après suppression; le contrôle « schedule déjà présent » ne protège que si B existe encore au moment de la restauration. Le test final ne couvre pas ce scénario et le correctif ne le ferme pas.

Une résolution fiable demanderait un état/génération de claim durable ou une règle d’annulation explicite pour les opérations déjà prises en charge, avec reprise/expiration cohérente. Ce lot ne modifie pas la politique. Le patch demeure une expérience Forgejo scratch, non adoptée; aucun clic navigateur, build complet ou test frontend n’est revendiqué. Ce test n’utilise aucun LLM/AgentOS/Forge Code/RAG/mémoire.
