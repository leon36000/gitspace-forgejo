# POC isolé — résidu LFS après refus CAS

Date : 24 octobre 2026. Statut : test de régression uniquement dans la copie jetable de Forgejo; aucun correctif Forgejo n’est adopté ni déployé.

## Résultat vérifié

Sur Forgejo v17.0.0 (`0d791421f85ca13a63b665053d81ea765fbbc571`) augmenté du patch CAS expérimental décrit dans [FORGEJO-BASE-CAS-POC-001](../forgejo-v17-base-cas-poc-001/local-run.md), le test déterministe arrête la PR cible après son fetch, fusionne une PR concurrente, puis libère la cible. L’API refuse cette dernière avec `409 merge push out of date`; la PR cible reste ouverte, le ref Git et la ligne de branche DB demeurent sur le commit concurrent. Pourtant, la préparation du merge a créé l’association LFS du dépôt cible avant l’échec du lease.

Le test utilise un dépôt cible public (`IsPrivate` vaut sa valeur par défaut `false`) et un pointeur LFS présent sur la tête de PR. Après le refus CAS, une requête GET anonyme au chemin LFS du dépôt cible répond `200` et retourne les 46 octets du contenu synthétique. Cela démontre que la métadonnée résiduelle publie l’objet sous le dépôt cible malgré la PR non fusionnée. Le contenu est synthétique et la cible est publique; ce résultat ne démontre donc pas la divulgation d’un contenu privé. Le comportement LFS d’un dépôt privé n’a pas été testé; le code vérifie les permissions de lecture du dépôt avant de servir l’objet.

La cause observée concorde avec l’ordre du code : `services/pull/merge.go` appelle `LFSPush` avant la commande Git avec `--force-with-lease`; `services/pull/lfs.go` crée l’association dans le dépôt cible; l’échec du push ne retire pas cette association. Le test ne modifie pas cette logique.

## Reproduction

Source officielle : clone propre d’audit `/tmp/codex-forgejo-v17.0.0-source`, tag `v17.0.0`, commit ci-dessus. Copie de travail scratch : `/tmp/forgejo-v17-base-cas-poc`, basée sur le même commit avec MERGE-011 et l’extension CAS expérimentale non suivie au produit.

Commande lancée depuis `/tmp/forgejo-v17-base-cas-poc` avec Go `go1.27.3 linux/arm64` :

```sh
/tmp/forgejo-merge011-test/toolchain/bin/go test -count=1 -v -timeout=20m -tags sqlite ./tests/integration -run '^TestPullMergeCASRefusalLeavesLFSMetadata$'
```

Code de sortie : `0`; `TestPullMergeCASRefusalLeavesLFSMetadata` et le paquet passent. La sortie du run individuel est dans [lfs-residue-download-test.log](lfs-residue-download-test.log). Le test source archivé est [lfs-residue-test-source.go](lfs-residue-test-source.go). Une relance consolidée des trois régressions CAS est également archivée dans [cas-regressions-final.log](cas-regressions-final.log). `git diff --check` passe dans le scratch Forgejo. Les seuls fichiers Forgejo modifiés dans cette sous-tâche sont des tests; aucune logique de production n’a été modifiée.

## Portée et remédiation à étudier

- L’effet établi est l’association résiduelle et la disponibilité par le dépôt cible public après un refus de fusion. L’impact sur la confidentialité dépend des ACL et de la visibilité du dépôt; aucun accès privé non autorisé n’est démontré.
- La correction devra rendre les associations LFS non visibles jusqu’à confirmation du lease, puis les publier de manière idempotente et récupérable après interruption. Un journal de reprise doit couvrir le cas où la ref a bougé mais la publication LFS n’a pas encore été finalisée. La suppression compensatoire seule est insuffisante sans prouver qu’aucune autre ref ne référence l’objet.
- Ce test ne qualifie pas tous les effets secondaires du merge, tous les styles ou les migrations; il ne transforme pas le patch CAS en changement adopté.
- Ne pas intégrer ce POC au produit. Forgejo v17 reste GPL-3.0-or-later; le fichier de test archivé conserve son en-tête `SPDX-License-Identifier: MIT`. Le statut de licence GitSpace demeure indéterminé.

Empreintes du lot consolidé sur la source finale : journal SHA-256 `c2eecb8671533431c161eb51c3ed9861c580f0ff1ff4166c0cd357657c2aab9d`; source SHA-256 `dc24f3b721ec3924818d9c58159edb84843ff6217f281a486c8779c58fe69e69`.

## Contrôle négatif — précondition de base omise

Le même test source exerce ensuite le contrat API : avec la base vérifiée obsolète, Forgejo répond `409`; la PR reste ouverte. Si l’appel legacy omet `expected_base_commit_id`, le serveur accepte le POST (`200`) et fusionne la PR contre la base plus récente. Le formulaire web avec token obsolète répond `400` avec « base branch no longer matches the verified commit », sans fusion ni déplacement de ref.

Un test web séparé confirme aussi l'omission : après une PR concurrente, le formulaire avec l'ancien SHA est d'abord sondé jusqu'au refus explicite; le formulaire suivant sans SHA répond `200`, fusionne et a pour premier parent la nouvelle base. `Mergeable=true` seul ne garantit pas que tous les contrôles web sont prêts; la source sonde donc avec le token périmé jusqu'au refus spécifique. Les sous-cas API AutoMerge/manually-merged refusent un token obsolète lorsqu'il est fourni.

Deux nouveaux sous-cas testent l'absence du champ après avance concurrente. L'API accepte la programmation AutoMerge (`201 Created`) sur la PR cible avec un appel qui ne transmet aucun `expected_base_commit_id`; la PR reste alors ouverte et le ref reste sur le commit concurrent. L'API accepte aussi `manually-merged` sans précondition (`200`) et marque la cible fusionnée avec le commit de la PR concurrente. Le readback Git montre que le fichier de la PR cible est absent du commit retenu, tandis que le fichier concurrent y est présent : le statut `merged` ne correspond donc pas au contenu réellement incorporé. Le dépôt de test a explicitement autorisé le style de fusion manuelle via l'API. Ce résultat concerne le chemin d'assertion manuel; il ne démontre pas un changement de ref par cette route. Ces tests caractérisent le patch expérimental et n'ajoutent aucune logique Forgejo au produit.

Commande lancée depuis `/tmp/forgejo-v17-base-cas-poc` :

```sh
/tmp/forgejo-merge011-test/toolchain/bin/go test -count=1 -v -timeout=20m -tags sqlite ./tests/integration -run '^TestPullMergeRejectsStaleClientBase$'
```

Code de sortie : `0`; les sous-cas API AutoMerge/manually-merged refusent le token obsolète. La relance consolidée finale des quatre fonctions ciblées passe (API stale/omit, Web stale/omit, AutoMerge/manual omit et LFS) :

```sh
/tmp/forgejo-merge011-test/toolchain/bin/go test -count=1 -v -timeout=20m -tags sqlite ./tests/integration -run '^(TestPullMergeCASRefusalLeavesLFSMetadata|TestPullMergeRejectsStaleClientBase|TestPullMergeWebRouteAcceptsMissingExpectedBase|TestPullMergeSpecialRoutesAcceptMissingExpectedBase)$'
```

Depuis `/tmp/forgejo-v17-base-cas-poc`, Go `go1.27.3 linux/arm64`, exit `0`; les quatre fonctions et le paquet passent. Le journal consolidé exact est [cas-regressions-final.log](cas-regressions-final.log), SHA-256 `17a524a582e71cea4aa7639b10df35d281b7e39690c3c905a9d58777665fbc8b`. La source complète des tests scratch est [pull-merge-freshness-race-test-source.go](pull-merge-freshness-race-test-source.go), SHA-256 `6144e757036a6341e494e9ba7eca81f53d37b60a813afa22cd2e802e34501fec`; `gofmt` et `git diff --check` passent. L'arbre scratch reste modifié par le patch expérimental et les tests; le clone officiel propre ne l'est pas.
