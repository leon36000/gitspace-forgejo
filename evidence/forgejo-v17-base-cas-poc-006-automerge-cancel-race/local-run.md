# FORGEJO-BASE-CAS-POC-006 — annulation avant prise en charge AutoMerge (24 octobre 2026)

## Base et périmètre

- Forgejo officiel v17.0.0, commit de base `0d791421f85ca13a63b665053d81ea765fbbc571`; code testé dans le scratch cumulatif `/tmp/forgejo-v17-base-cas-poc`, sans commit de travail et sans copie dans le produit.
- Toolchain Go `1.27.3 linux/arm64`, tests SQLite, `GOCACHE=/dev/shm/codex-forgejo-v17-go-cache`, `TMPDIR=/dev/shm`, `GOMAXPROCS=4`.
- Le test cible est `TestPullMergeAutoMergeCancellationBeforeScheduleClaimWins` dans `tests/integration/pull_merge_freshness_race_test.go`; il suspend le worker après ses vérifications, annule le schedule par DELETE API autorisé (`204`), puis reprend le worker.
- Les empreintes SHA-256 des trois fichiers scratch concernés dans l’état final POC-007 figurent dans `scratch-source-sha256.txt`.

## Résultat rouge et correction

Le test rouge a reproduit le défaut : après le `204`, le worker ignorait l’absence du schedule et fusionnait quand même la PR. La correction arrête le worker si la suppression de la planification échoue ou si l’entrée est déjà absente. Le test vert confirme que le schedule reste absent, la PR reste ouverte et la branche de base ne change pas.

La revue indépendante a ensuite trouvé qu’un worker ancien pouvait encore supprimer une nouvelle planification créée après l’annulation, car la suppression par PR ID ne distinguait pas les générations. Cette suite a été traitée en POC-007 avec une suppression conditionnelle sur l’ID du schedule; le run final POC-007 réexécute aussi le test POC-006.

## Vérifications

- Rouge : `automerge-cancel-race-red.log`, test en échec car `HasMerged=true` après un DELETE `204`.
- Vert initial : `automerge-cancel-race-green.log`, tests POC-005/006 sélectionnés, exit 0.
- Régression finale après durcissement par ID : `../forgejo-v17-base-cas-poc-007-automerge-replacement-race/automerge-regressions-final.log`, 8 fonctions d’intégration AutoMerge/politique passent en `24.272s`.
- Packages `./services/automerge ./services/pull` passent après le durcissement POC-007 (`service-tests.log` du POC-007). `git diff --check` est propre.

Commande du test rouge/vert ciblé :

```sh
GOCACHE=/dev/shm/codex-forgejo-v17-go-cache TMPDIR=/dev/shm GOMAXPROCS=4 \
  /tmp/forgejo-merge011-test/toolchain/bin/go test -count=1 -v -timeout=20m -tags sqlite \
  ./tests/integration -run '^TestPullMergeAutoMergeCancellationBeforeScheduleClaimWins$'
```

## Limites

Ce correctif reste un POC scratch, non adopté au produit. Il traite l’annulation gagnant avant la prise en charge; il ne prouve pas l’annulation d’une fusion déjà prise en charge, ni l’atomicité entre changement de politique, claim et fusion. La restauration après refus de politique conserve encore une course documentée en POC-007. Pas de test de clic navigateur, de build complet Forgejo ni de test frontend. Aucun LLM ou AgentOS n’est utilisé dans ce test.
