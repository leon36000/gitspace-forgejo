# POC isolé — précondition de base CAS pour Forgejo v17.0.0

Date : 24 octobre 2026. Statut : expérimentation dans une copie temporaire du code Forgejo, non intégrée au produit, non déployée.

## Question et résultat

Le gate local lit la tête et la base d’une PR avant le POST de fusion. Au tag source v17.0.0, l’API n’accepte que `head_commit_id`. Forgejo vérifie sa propre tête et son push refuse un déplacement de ref après son fetch, mais le gate ne peut pas lier son commit de base vérifié à ce fetch. Un writer peut donc avancer la base après le dernier GET du gate et avant la lecture faite par Forgejo.

Une extension scratch ajoute `expected_base_commit_id` au formulaire et au chemin synchrone API/Web : le serveur le compare au SHA de base lu pendant le contrôle, vérifie à nouveau le SHA Git effectivement fetché et protège l’écriture finale par `--force-with-lease`. Le résultat de fusion doit avoir le SHA attendu comme ancêtre. En cas de base cliente périmée, l’API répond `409`, la PR cible reste ouverte et le ref n’avance pas. Si la base change entre le fetch interne et le push, le lease refuse l’écriture.

Le champ est **optionnel** pour conserver la compatibilité. Cette option ne protège donc que les appels qui l’envoient; elle n’empêche ni un client autorisé de l’omettre, ni un autre chemin d’écriture de contourner cette précondition. Ce n’est pas une protection générale activable par défaut.

## Sources et provenance

- Dépôt officiel Forgejo : `https://codeberg.org/forgejo/forgejo`, tag `v17.0.0`, commit source `0d791421f85ca13a63b665053d81ea765fbbc571`. Forgejo publie v17.0.0 le 15 octobre 2026; la page officielle de version est [Forgejo v17.0](https://forgejo.org/2026-10-release-v17-0/). L’extraction Web de Codeberg est bloquée par robots.txt; l’audit de code provient du clone local épinglé au commit ci-dessus.
- Clone d’audit officiel `/tmp/codex-forgejo-v17.0.0-source` propre après l’essai. Copie de travail modifiée `/tmp/forgejo-v17-base-cas-poc`, HEAD inchangé au commit v17.0.0.
- La copie scratch contient le patch complet MERGE-011 (`/tmp/forgejo-merge011-fix-test/merge-011-expected-base.patch`, SHA-256 `fa4a6ac37c50fc1a60cd7a7cc98ea9c7fb71cf1d611e79c635585643ca81c6e6`) et l’extension API/Web décrite ici. Le hash du diff scratch suivi depuis HEAD est `10e444bc8aba9ee0c0bae8fc2ba54f1a265b8986bbf51d5e028f179fb2097286`; l’empreinte des fichiers modifiés, dont le test non suivi par Git, est dans [scratch-source-files.sha256](scratch-source-files.sha256).
- Forgejo est GPL-3.0-or-later au fichier `LICENSE` de cette source. Aucun code Forgejo n’est copié dans le worktree produit; seul ce compte rendu et les sorties de test y sont conservés. Le statut de licence de GitSpace reste indéterminé et aucune de ses sources n’est copiée.

## Vérifications effectuées

Environnement : Linux ARM64; Go `go1.27.3` depuis `/tmp/forgejo-merge011-test/toolchain/bin/go`; configuration d’intégration SQLite locale générée dans le clone scratch. Les dépendances Go épinglées manquantes ont été téléchargées depuis leurs sources publiques pendant la compilation; aucune donnée privée ni aucun credential produit n’a été envoyé à un service externe.

Depuis `/tmp/forgejo-v17-base-cas-poc` :

```sh
/tmp/forgejo-merge011-test/toolchain/bin/go test -count=1 -v -timeout=30m -tags 'sqlite sqlite_unlock_notify' ./tests/integration -run '^(TestPullMerge|TestPullRebase|TestPullRebaseMerge|TestPullSquash|TestFastForwardOnlyMerge|TestCantFastForwardOnlyMergeDiverging|TestPullAutoMergeAfterCommitStatusSucceed|TestPullMergeBaseFreshnessTOCTOU|TestPullMergeBaseFreshnessAfterBranchSync|TestPullMergeBaseFreshnessAfterFetch|TestPullMergeRejectsStaleClientBase)$'
```

Code de sortie Go : 0. Le journal de cette exécution directe [selected-tests.log](selected-tests.log) montre 11 fonctions de test au niveau supérieur en `PASS`, dont les scénarios de changement de base avant le contrôle, après synchronisation DB, après fetch, et le client présentant une base périmée. Dans ce dernier cas, le merge concurrent réussit, le merge cible reçoit `409`, et le readback laisse sa PR ouverte. Le même groupe couvre merge, rebase, rebase-merge, squash, fast-forward, fast-forward refusé et AutoMerge existant.

```sh
/tmp/forgejo-merge011-test/toolchain/bin/go test -count=1 -timeout=30m -tags 'sqlite sqlite_unlock_notify' ./services/pull
```

Code de sortie Go : 0; sortie dans [services-pull.log](services-pull.log). `git diff --check` retourne 0 dans le clone scratch.

Le manifest local [SHA256SUMS](SHA256SUMS) fixe ces journaux et le rapport. L’empreinte ne prouve que l’intégrité des artefacts présents.

## Portée et risques non couverts

- Ce n’est pas la suite Forgejo complète : seuls le groupe d’intégration nommé et le paquet `services/pull` ont été exécutés. Il n’y a pas de suite de tests HTTP ciblée pour le formulaire Web; ses chemins ont été compilés par les tests, pas vérifiés en comportement.
- Le nouveau test client valide l’API synchrone. Les cas où le champ est présent sont refusés pour AutoMerge différé et « manually merged »; les autres modes continuent à exister sans le champ. AutoMerge, protection de branches, installation et mise à niveau ne sont pas qualifiés globalement.
- **LFS n’est pas couvert par ces tests.** La revue du diff a constaté que `LFSPush` associe les métadonnées LFS à la base avant le push protégé par lease (`services/pull/merge.go` et `services/pull/lfs.go`). Un refus CAS peut donc laisser une association LFS latérale. Il faut déplacer/rendre transactionnel cet effet ou prouver son innocuité, puis ajouter un test.
- Le modèle de concurrence ne traite pas tous les écrivains, les forks, les événements secondaires, les annulations, ni les échecs après écriture. L’autorité et l’identité de fusion doivent être verrouillées; l’option est de créer une exigence serveur, un endpoint dédié ou une politique de dépôt qui interdit le merge sans précondition.
- Le patch ajoute une surface de maintenance Forgejo GPL. Le coût des rebasages et du support upstream n’est pas estimé. Le POC ne justifie pas encore l’adoption d’un fork.
- Une première version du test a échoué avant l’ajout de la comparaison du SHA fourni par le client; sa sortie brute n’a pas été archivée. Elle n’est pas comptée comme une preuve reproductible dans ce rapport.
- Aucun binaire produit ni instance de production n’a été modifié. Le binaire local v17 déjà utilisé dans d’autres parcours n’a toujours pas de provenance de build reliée à ce clone source.

