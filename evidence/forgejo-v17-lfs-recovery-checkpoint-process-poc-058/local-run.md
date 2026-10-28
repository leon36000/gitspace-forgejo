# POC-058 — reprise du checkpoint LFS par processus indépendants

Date : 25 octobre 2026. Scratch : `/tmp/forgejo-v17-base-cas-poc`, source officielle Forgejo `v17.0.0`, HEAD de base `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch contient d’autres POC expérimentaux non propres; ils n’ont pas été nettoyés. Aucun code Forgejo n’a été copié au produit.

## Scénario

Le test sous build tag `forgejo_poc_lfs_recovery_multiprocess` crée une base SQLite fichier, un dépôt Git nu réel et trois associations pending. Il lance trois nouveaux processus du binaire de test, chacun ouvrant la même base et traitant une page de taille 1 avec `RecoverLFSPendingMetaObjectsCheckpointPage`. Le parent vérifie dans la base après chaque sortie que le curseur durable avance sous la borne du snapshot; le troisième processus clôt le cycle, remet le curseur et la borne à zéro, puis le parent relit les trois associations actives et constate qu’aucune ne reste pending.

Cela établit que le curseur n’est pas seulement en mémoire du premier processus : les deux processus suivants le relisent et poursuivent la génération. Chaque enfant s’arrête normalement après une page; le test ne tue pas un processus en plein appel.

## Essai et correction du harnais

Le premier run a échoué sur l’assertion de sortie `page=2`. La sortie de l’enfant indiquait `page=1`, calculé à tort comme `SweepGeneration+1`; la génération ne change qu’à la fin du cycle, et ne compte pas les pages. Le test avait déjà observé le second enfant terminer sa page et échouait uniquement sur cette assertion de diagnostic, pas sur le checkpoint. L’assertion a été remplacée par l’état attendu `complete=false/true`; les transitions du curseur, de la borne et de génération restent lues dans SQLite par le parent. L’échec initial est conservé dans `red-green.log`.

Commande finale, avec dépendances locales seulement :

```text
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags sqlite,forgejo_poc_lfs_recovery_multiprocess ./services/pull -run '^TestPOCLFSPendingRecoveryCheckpointResumesAcrossProcesses$' -count=1 -v -timeout=10m
```

Résultat : `PASS`, test 1/1, paquet `services/pull` réussi en 1,466 s. `gofmt -d services/pull/lfs_recovery_multiprocess_poc_test.go` est vide et `git diff --check` passe dans le scratch. Les sorties sont dans `process-resume-final.log`, `gofmt-diff.log` et `scratch-diff-check.log`.

Environnement : Linux/aarch64, Go `1.27.3`, Git `2.43.0`; détails et SHA-256 des sources (`models/git/lfs.go`, `services/pull/lfs_recovery.go`, tests et manifestes Go) dans `tool-versions.txt` et `source-hashes.txt`.

## Portée et limites

- Test de fonction dans trois processus enfants séquentiels partageant un SQLite fichier et un dépôt Git nu sur le même hôte; aucun serveur Forgejo n’est démarré, aucun processus n’est tué par SIGKILL et aucun worker d’exploitation n’est branché.
- La base est SQLite seulement; aucune preuve PostgreSQL/MySQL, multi-hôte, écrivains concurrents sur le même checkpoint, ordre migration/synchronisation au démarrage, ou ordonnanceur équitable entre dépôts.
- Ce test prolonge le POC-057; il ne valide pas la promotion après déplacement concurrent du ref Git entre l’observation et l’écriture des métadonnées.
- Le code et les tests LFS restent dans le scratch non propre, sans adoption, publication ni copie au produit. Aucun LLM, modèle, secret ou service externe n’est utilisé.
- Il s’agit d’un test ciblé unique (`n=1`), pas d’un benchmark. Le parcours LLM, la comparaison contrôlée avec Forgejo+agent+CI conventionnels et plusieurs refus composites du produit restent non démontrés.

Prochaine action : inspecter le chemin de démarrage/migration réel qui synchroniserait ce modèle, puis déterminer et tester le risque de changement de ref avant promotion. Évaluer l’équité inter-dépôts et la rétention/nettoyage avant de proposer un worker. En parallèle, poursuivre seulement les lacunes de refus produit non encore couvertes.
