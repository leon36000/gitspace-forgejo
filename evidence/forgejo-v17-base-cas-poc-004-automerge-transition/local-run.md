# FORGEJO-BASE-CAS-POC-004 — transition AutoMerge déjà planifié (24 octobre 2026)

## Base examinée et périmètre

- Source officielle Forgejo v17.0.0, tag `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; clone officiel propre conservé dans `/tmp/codex-forgejo-v17.0.0-source`.
- Copie scratch cumulative dans `/tmp/forgejo-v17-base-cas-poc`, sans commit de travail. Elle porte les expérimentations POC-001 à POC-004; son HEAD reste le commit officiel ci-dessus. Aucun code scratch n’est copié au dépôt produit.
- Licence du tag : GNU GPL-3.0. Le statut de licence GitSpace reste à établir et n’est pas impliqué par ce test.
- Outils : Go `1.27.3 linux/arm64`; tests SQLite, cache Go sous `/dev/shm`.

## Défaut reproduit et changement scratch

Le test planifie d’abord AutoMerge en mode compatibilité, active ensuite `RequireExpectedBaseCommitID`, puis fait exécuter `StartPRCheckAndAutoMerge`. Avant le correctif, `handlePullRequestAutoMerge` effaçait la ligne de planification avant l’appel central à `Merge`; celui-ci refusait la fusion faute de précondition synchrone. La branche restait inchangée, mais la planification disparaissait. Le journal rouge montre ce refus et l’assertion échouée sur la planification manquante.

Le changement dans `services/automerge/automerge.go` vérifie la politique courante avant les contrôles de la PR et avant suppression de la planification. Si la politique exige une base vérifiée synchrone, le worker garde la planification et journalise qu’elle peut être annulée ou que la politique peut être désactivée. Pour les cas AutoMerge qui restent admissibles, le worker conserve aussi le SHA retourné par `CheckPullMergeableWithBase` et le transmet à `Merge`.

Le test vert vérifie que la PR reste ouverte, que la branche de base ne bouge pas, que la planification subsiste, puis que l’annulation API autorisée renvoie `204` et retire cette planification. Le formulaire Vue présente une annulation quand une planification existe (`web_src/js/components/PullRequestMergeForm.vue:192`), servie par la route Web `CancelAutoMergePullRequest` (`routers/web/repo/pull.go:1629`). L’intégration exercée ici annule par API; le clic navigateur n’a pas été testé.

## Vérifications

Test rouge avant le correctif :

```sh
GOCACHE=/dev/shm/codex-forgejo-v17-go-cache TMPDIR=/dev/shm GOMAXPROCS=4 \
  /tmp/forgejo-merge011-test/toolchain/bin/go test -ldflags='-s -w' -count=1 \
  -timeout=20m -tags sqlite ./tests/integration \
  -run '^TestPullMergeRepositoryPolicyPreservesPreviouslyScheduledAutoMergeForCancellation$'
```

Résultat : échec attendu, car la planification avait été effacée (`automerge-transition-red.log`). Après le correctif, la même commande passe : `ok forgejo.org/tests/integration 2.101s` (`automerge-transition-green.log`).

Tests de services :

```sh
GOCACHE=/dev/shm/codex-forgejo-v17-go-cache TMPDIR=/dev/shm GOMAXPROCS=4 \
  /tmp/forgejo-merge011-test/toolchain/bin/go test -count=1 -timeout=20m \
  -tags sqlite ./services/automerge ./services/pull
```

Résultat : les deux packages passent (`0.240s` et `0.398s`, `service-tests.log`). Une sélection de 11 tests d’intégration CAS/politique/fraîcheur/LFS passe en `29.963s` (`integration-regressions.log`). `git diff --check` passe. Aucun build complet Forgejo ni test frontend n’est revendiqué.

Les empreintes des 23 fichiers scratch modifiés/non suivis au moment de cet instantané sont dans `scratch-source-sha256.txt`; ce manifeste couvre l’état cumulatif POC-001..004, pas seulement le delta 004. POC-003 garde son propre instantané antérieur.

## Limites et décision

- La vérification de politique et la suppression de planification ne sont pas atomiques avec une modification concurrente des réglages. Si la politique bascule après le contrôle du worker, une course peut encore mener au refus de `Merge` après suppression de la planification. Aucun test n’exerce cette course; aucune garantie d’atomicité n’est revendiquée.
- Avec la politique stricte, une planification préexistante est conservée mais ne sera pas exécutée tant que la politique reste active. La page permet de l’annuler, mais son message n’explique pas encore explicitement ce blocage; un état métier visible/actionnable serait préférable avant adoption.
- POC-002 démontre toujours un résidu de métadonnées LFS après refus CAS. POC-004 ne le corrige pas.
- Le SHA client vérifie la fraîcheur de la base, pas l’exécution indépendante des critères. Les rôles d’exécution, de vérification et d’autorité de fusion doivent rester séparés.
- Ceci reste une expérience scratch sur Forgejo v17, non un patch adopté ou déployé, et ne démontre aucun gain du parcours avec LLM/AgentOS.
