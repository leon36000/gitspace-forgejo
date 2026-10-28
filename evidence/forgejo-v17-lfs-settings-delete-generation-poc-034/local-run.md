# POC-034 — suppression LFS via la route Settings avec mapping de génération

Date : 24 octobre 2026. Sources auditées : Forgejo `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch `/tmp/forgejo-v17-base-cas-poc` reste modifié par les POC précédents. Ce lot ajoute un test d’intégration sous build tag uniquement; aucun fichier de production Forgejo n’a été modifié.

## Résultat

Deux scénarios traversent la route web authentifiée `POST /{owner}/{repo}/settings/lfs/delete/{oid}` et la fonction `LFSDelete` de Forgejo. L’adaptateur de stockage générationnel reste une fixture de test avec SQLite et `LocalStorage`.

- Suppression normale : la route appelle `Delete` avec le chemin logique OID. L’adaptateur le résout vers la génération courante, supprime le blob puis retire le mapping; Forgejo répond `303 See Other` et le test confirme l’absence du mapping et du fichier générationnel.
- Reprise après erreur : la première suppression physique reçoit une erreur injectée. La route répond `500`; l’association Forgejo a déjà été retirée, mais le mapping et le blob restent présents. Un second POST au même OID est accepté, répond `303` et termine la suppression.

Le code Forgejo appelé sépare bien la suppression de l’association et celle du blob : `models/git/lfs.go:RemoveLFSMetaObjectByOidFn` commet la transaction d’association avant que `routers/web/repo/setting/lfs.go:LFSDelete` appelle le stockage. Le `FIXME` du handler signale aussi l’absence de verrouillage. Ce POC vérifie seulement qu’un adaptateur peut résoudre le chemin logique et qu’un nouvel appel permet de reprendre après une erreur. Il ne corrige pas ces frontières dans Forgejo.

## Commande et vérification

Forgejo `v17.0.0`, Go `1.27.3 linux/arm64`, XORM `v1.4.1`, SQLite `v1.14.52`. Le dernier run combiné sélectionne six fonctions de test POC-031/032/033/034 (le test de rejet inclut deux sous-tests) et termine avec le code 0 :

```sh
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_route forgejo_poc_sqlite_generation_upload_route' \
  ./tests/integration \
  -run '^TestPOCLFS(AuthenticatedUploadPromotesSQLiteGenerationAndReadsItBack|UploadRetryPromotesDurablePendingGeneration|UploadSurvivesProcessRestart|InvalidUploadRejectsStageThenFreshAttemptRecovers|SettingsDeleteResolvesCurrentGeneration|SettingsDeleteCanRetryAfterStorageFailure)$' \
  -count=1 -v
```

La sortie est conservée dans `final-test.log`. Les lignes `testlogger.go:recordError() FATAL ERROR` correspondent aux appels `log.Error` attendus pour la panne injectée et les refus de hash/taille des POC antérieurs; Forgejo commente l’appel `t.Errorf` associé dans son testlogger. Les assertions, les statuts HTTP et l’exit code 0 sont les observations utilisées.

Un essai intermédiaire avait enregistré les nettoyages de test dans un ordre qui déclenchait `Flushing queues failed with error context canceled`; son journal est conservé sous `test-cleanup-order-red.log`. Le test a été corrigé pour restaurer le stockage avant le teardown Forgejo. Les deux tests de suppression puis la commande combinée finale passent après cette correction.

## Limites

La génération, sa table SQLite et sa résolution sont ajoutées par le test; Forgejo v17 n’a pas ce mapping en production. Le scénario ne teste pas les associations multi-dépôts, les courses entre publication et suppression, la suppression après crash physique, les URLs, Doctor/GC, `Resolve/Open` en route, migration/dump, S3/MinIO ou un client `git-lfs`. L’échec initial de suppression laisse une association retirée; seule la conservation du mapping/blob dans l’adaptateur et la reprise par un second appel sont démontrées. Aucun changement de Forgejo, fork, adoption, parcours LLM ni comparaison de valeur n’est démontré.

## Suite proposée

Raccorder dans un prototype de test l’URL de téléchargement au mapping de génération et comparer `Resolve/Open` à la route LFS réelle; poursuivre ensuite l’audit des lecteurs, suppressions, Doctor/GC, migrations et dump. Toute proposition de modification upstream doit encore chiffrer la compatibilité et la maintenance.
