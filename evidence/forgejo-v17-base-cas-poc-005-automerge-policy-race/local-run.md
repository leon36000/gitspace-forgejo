# FORGEJO-BASE-CAS-POC-005 — bascule de politique pendant AutoMerge (24 octobre 2026)

## Base et périmètre

- Forgejo officiel v17.0.0, commit de base `0d791421f85ca13a63b665053d81ea765fbbc571`; le clone officiel `/tmp/codex-forgejo-v17.0.0-source` reste propre.
- Copie scratch cumulative `/tmp/forgejo-v17-base-cas-poc`, sans commit de travail. Aucun fichier Forgejo n’est repris dans le dépôt produit. Licence de la base : GPL-3.0.
- Toolchain : Go `1.27.3 linux/arm64`; SQLite; `GOCACHE=/dev/shm/codex-forgejo-v17-go-cache`, `TMPDIR=/dev/shm`, `GOMAXPROCS=4`.

## Reproduction rouge et correction

Le test `TestPullMergeRepositoryPolicyRacePreservesScheduledAutoMerge` place un seam juste après les contrôles de mergeabilité et avant la suppression du schedule. Il programme AutoMerge sous la politique historique, laisse le worker atteindre ce seam, active `RequireExpectedBaseCommitID`, puis le libère. La version rouge exécute la fusion, reçoit `ErrExpectedBaseCommitRequired`, mais avait déjà supprimé le schedule; le test échoue précisément sur l’attente que ce schedule reste annulable. La PR demeure ouverte et la base ne bouge pas.

Le scratch restaure maintenant la planification originale quand `Merge` refuse spécifiquement avec `ErrExpectedBaseCommitRequired`; les autres erreurs gardent le comportement préexistant de suppression. Le test vert vérifie la planification restaurée, la PR toujours ouverte, la base inchangée et une annulation autorisée par DELETE API (`204`). Les avertissements du worker précisent que désactiver la politique demande ensuite de recréer la planification.

## Vérifications

Test rouge et vert :

```sh
GOCACHE=/dev/shm/codex-forgejo-v17-go-cache TMPDIR=/dev/shm GOMAXPROCS=4 \
  /tmp/forgejo-merge011-test/toolchain/bin/go test -v -ldflags='-s -w' -count=1 \
  -timeout=20m -tags sqlite ./tests/integration \
  -run '^TestPullMergeRepositoryPolicyRacePreservesScheduledAutoMerge$'
```

La sortie rouge attendue est dans `automerge-policy-race-red.log`; le succès vert dans `automerge-policy-race-green.log` (`PASS`, test 1.85s, paquet 2.276s). Une première version du harnais appelait `StartPRCheckAndAutoMerge` depuis le goroutine de test, alors que `queue.Push` exécutait le handler inline : la pile conservée dans `automerge-policy-race-harness-hang.log` montre le blocage au seam. Le harnais a été corrigé en démarrant ce déclencheur dans un goroutine séparé. Cette capture n’est pas le test rouge comportemental.

Les tests `./services/automerge ./services/pull` passent (`0.234s`, `0.396s`). Une sélection finale de 12 fonctions d’intégration couvrant les préconditions CAS, les routes, la politique stricte, les races de base, AutoMerge préexistant et le résidu LFS passe en `31.959s`. Résultats : `service-tests.log`, `integration-regressions.log`. `gofmt` et `git diff --check` passent. Aucun build complet Forgejo ni test frontend n’est revendiqué.

`scratch-source-sha256.txt` contient les empreintes de 23 fichiers scratch modifiés/non suivis; c’est l’état cumulatif POC-001..005, y compris le test d’intégration.

## Limites

- La suppression initiale et la restauration sont des écritures séparées. Un arrêt du processus exactement entre elles peut encore perdre le schedule; aucun test de crash à cet endroit n’a été fait.
- Quand la politique est désactivée, le handler de réglages ne réenfile pas à lui seul tous les schedules existants. Pour reprendre, il faut désactiver la politique, annuler puis recréer la planification (ou provoquer un événement existant qui réenfile cette PR). Aucun E2E de redéclenchement n’est revendiqué.
- La page montre le bouton d’annulation d’un schedule existant mais son message reste générique; le clic navigateur n’est pas testé.
- Le résidu de métadonnées LFS après refus CAS observé en POC-002 demeure. Le SHA de base reste une précondition de fraîcheur, pas une attestation d’exécution indépendante.
- L’expérience reste scratch et ne ferme pas la frontière d’autorité de fusion. Le parcours produit utilise l’adaptateur local déterministe, sans LLM; aucun gain de valeur n’est mesuré.
