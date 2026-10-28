# FORGEJO-BASE-CAS-POC-003 — opt-in du SHA de base et parcours Web (24 octobre 2026)

## Base examinée et périmètre

- Dépôt officiel : [Forgejo](https://codeberg.org/forgejo/forgejo), tag `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571` (daté du 15 octobre 2026). Le clone propre `/tmp/codex-forgejo-v17.0.0-source` est resté sans modification. La copie jetable `/tmp/forgejo-v17-base-cas-poc` est basée sur ce commit et contient le patch expérimental cumulatif des POC-001/002/003; elle n’a pas de commit de travail.
- La licence du tag est GNU GPL version 3; aucune source Forgejo n’a été copiée dans le dépôt produit. GitSpace reste hors de ce patch et son statut de licence reste à établir.
- Toolchain de test : Go `1.27.3 linux/arm64`; `GOCACHE=/dev/shm/codex-forgejo-v17-go-cache`, `TMPDIR=/dev/shm`, `GOMAXPROCS=4`. Le test JavaScript utilise Node `v24.22.0` et npm `11.19.1`, mais n’a pas pu être exécuté (voir limites).

## Changement scratch

L’option de dépôt `require_expected_base_commit_id` reste désactivée par défaut. Une mise à jour de dépôt par API peut l’activer; le champ est ensuite lisible au GET. Sous cette option, les fusions API et Web synchrones exigent `expected_base_commit_id`, qui doit égaler le SHA renvoyé par `CheckPullMergeableWithBase`. La couche service refuse également l’appel sans précondition cliente. Le SHA contrôlé est conservé jusqu’au fetch et au push avec `--force-with-lease`; le résultat est également vérifié comme descendant de cette base.

La page Web rend le SHA de la branche de base observé au chargement et le formulaire Vue le renvoie dans un champ caché. Une course après l’affichage donne donc un refus, pas une fusion sur une base plus récente. En mode strict, l’interface masque la fusion différée et le marquage manuel; les routes et la couche service les refusent également, car ces chemins ne consomment pas une précondition synchronisée avec l’autorité externe.

Ce champ est un contrôle de fraîcheur, pas une preuve que des critères indépendants ont été exécutés. Un client autorisé peut fournir lui-même le SHA courant. L’autorité de vérification doit donc rester séparée et seule habilitée à appeler l’autorité de fusion; ce patch ne crée ni permis signé, ni identité de vérificateur, ni preuve de correction.

## Vérifications exécutées

Suite d’intégration ciblée, 10 fonctions sélectionnées, dont refus API/Web sur base périmée, absence de token, routes différées/manuelles, politique stricte, fusion Web avec le SHA rendu, trois courses de fraîcheur et résidu LFS :

```sh
GOCACHE=/dev/shm/codex-forgejo-v17-go-cache TMPDIR=/dev/shm GOMAXPROCS=4 \
  /tmp/forgejo-merge011-test/toolchain/bin/go test -ldflags='-s -w' -count=1 \
  -timeout=20m -tags sqlite ./tests/integration \
  -run '^TestPullMerge(RejectsStaleClientBase|WebRouteAcceptsMissingExpectedBase|SpecialRoutesAcceptMissingExpectedBase|RepositoryPolicyRequiresExpectedBase|RepositoryPolicyRejectsDeferredAndManual|RepositoryPolicyAcceptsWebMergeWithExpectedBase|BaseFreshnessTOCTOU|BaseFreshnessAfterBranchSync|BaseFreshnessAfterFetch|CASRefusalLeavesLFSMetadata)$'
```

Résultat : `ok forgejo.org/tests/integration 27.491s`, code 0. La fonction Web confirme aussi que la page rend `pullBaseCommitID` et le SHA courant, puis observe le refus sans token et la fusion avec le token. Rapport brut : [repo-policy-regressions.log](repo-policy-regressions.log).

Suite complète du package service :

```sh
GOCACHE=/dev/shm/codex-forgejo-v17-go-cache TMPDIR=/dev/shm GOMAXPROCS=4 \
  /tmp/forgejo-merge011-test/toolchain/bin/go test -count=1 -timeout=20m \
  -tags sqlite ./services/pull
```

Résultat : `ok forgejo.org/services/pull 0.390s`, code 0. Un premier lancement a révélé un appel de test resté à l’ancienne signature de `Merge`; il a été corrigé avant cette réussite. La tentative initiale est conservée sous [service-pull-tests-initial-callsite-failure.log](service-pull-tests-initial-callsite-failure.log).

`git diff --check` passe. Le JSON Swagger a été analysé après substitution de ses deux variables de template, et contient les champs de précondition de fusion et de politique de dépôt. `make swagger-check` n’a pas été lancé; l’outil générateur complet n’est donc pas validé.

## Limites et décision

- L’exécution frontend `npm exec --offline -- vitest run web_src/js/components/PullRequestMergeForm.test.js` n’a pas abouti : `node_modules` est absent et le paquet n’est pas dans le cache hors ligne. Aucun accès réseau ni installation n’a été fait. Le test Vue est écrit, mais pas exécuté; le test d’intégration couvre le rendu serveur et la route Web, pas le navigateur réel.
- Le changement n’est ni un build autonome de Forgejo, ni un déploiement. Il ne teste pas l’activation de la règle alors qu’une fusion automatique est déjà planifiée : le worker peut supprimer la planification avant le refus central, avec seulement un log. Ce comportement est sûr pour la branche, mais sans état explicatif visible.
- Le résidu de métadonnées LFS après refus CAS reste reproductible, comme documenté dans le POC-002; il n’est pas corrigé par ce lot.
- La valeur `expected_base_commit_id` n’est pas un secret ou un justificatif d’autorisation. Elle ne ferme pas une frontière de privilèges si l’agent peut lui-même appeler Forgejo pour fusionner. Aucun changement n’est adopté dans le produit; ce POC sert à étayer le contrat d’adaptateur et le coût d’un éventuel fork limité.
- Ces tests ne démontrent aucun parcours avec LLM, AgentOS, Forge Code, RAG ou mémoire, ni gain comparatif de coût ou de réussite.

Les empreintes SHA-256 des 23 fichiers modifiés de la copie scratch sont dans [scratch-source-sha256.txt](scratch-source-sha256.txt). Les sorties initiales et finales sont conservées dans ce dossier.
