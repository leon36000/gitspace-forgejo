# CONTINUATION-326 — cartographie ciblée des writers Forgejo v17

Date : 27 octobre 2026.

## Portée et identité de la source

Lecture statique des blobs du dépôt officiel Forgejo, tag v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le dépôt de travail `/tmp/forgejo-v17-base-cas-poc` est fortement modifié : `git status --short` montre 20 fichiers suivis modifiés et 43 éléments non suivis; les chemins audités incluent notamment `routers/api/v1/repo/file.go` et `services/repository/files/update.go`, qui sont modifiés dans ce scratch. Afin d’éviter de confondre les POC et le code amont, chaque lecture a utilisé `git show HEAD:<chemin>` et chaque recherche `git grep ... HEAD`. Les blobs lus et leurs identifiants Git sont conservés dans `source-blobs.txt`.

Aucun code, test ou configuration Forgejo n’a été modifié dans ce lot. Aucun test Go, binaire ou serveur n’a été lancé. Les résultats runtime cités ci-dessous sont des preuves antérieures distinctes, et non des tests rejoués ici.

## Cartographie constatée

### Fusion de PR et contrôles au hook

Les routes API et Web normales appellent le service de merge après `CheckPullMergeable`; AutoMerge relance des contrôles dans son worker avant d’appeler le même service. Le merge service transmet la tête attendue mais, selon l’audit CONT-315/319, ne transmet pas une version attendue de la branche de base ni une époque de politique atomique. Les expériences CONT-316/317/322/323 ont déjà observé des fusions après modification de politique ou révocation dans les intervalles de concurrence instrumentés. Le hook `preReceiveBranch` est donc une garde d’un chemin d’écriture, pas une transaction qui lie à elle seule la policy et la mise à jour Git.

Les pushes Git génériques passent par le pre-receive et ses contrôles selon le contexte de l’acteur et du push. La lecture de `preReceiveBranch` montre toutefois des exceptions de rôle/contexte relevées en CONT-321 et CONT-008; le chemin interne marqué `GITEA_INTERNAL_PUSH=true`, les tags et le wiki ne constituent pas une preuve de couverture universelle.

### Écritures de fichiers, patch et upload

Les routes REST create/update/delete de fichiers, le changement groupé de fichiers et ApplyDiffPatch aboutissent aux services de fichiers. Pour une mise à jour sur la même branche, `ChangeRepoFiles` appelle `VerifyBranchProtection` avant de préparer l’index et le commit, puis `TemporaryUploadRepository.Push` envoie le commit au dépôt cible avec `PushingEnvironment`; le code de Push précise explicitement qu’il appelle les hooks. La vérification de fichier/commit et la mutation de ref restent deux étapes séparées. Pour un nouveau nom de branche, la vérification `VerifyBranchProtection` du service commun n’est appelée que lorsque l’ancienne et la nouvelle branche sont identiques; la création finale est un push, dont la couverture dépend donc du hook.

`UploadRepoFiles` crée aussi un commit temporaire puis utilise `TemporaryUploadRepository.Push`. Le code de ce service ne montre pas le même appel à `VerifyBranchProtection` que `ChangeRepoFiles`; la route Web fait un contrôle de droit de commit préalable et le push appelle les hooks. Les autres routes API/Web de contenu et le patch suivent les mêmes primitives de push, mais aucune conclusion de couverture atomique n’en découle. L’ancien SHA de fichier ou `LastCommitID` protège certains conflits de contenu; ce n’est pas le token attendu de policy/ref consommé atomiquement par tous les writers.

Cela concorde avec les tests antérieurs : POC-061 et CONT-008 ont observé le refus de l’écriture fichier par le hook activé, mais aussi des limites de compatibilité et des résidus LFS. Ces essais ne démontrent ni une publication/récupération LFS atomique, ni un worker de réparation autonome.

### Création, suppression et renommage de branches

La création de branche API/Web utilise `CreateNewBranchFromCommit`, qui fait un push Git local et déclenche les hooks. La restauration Web d’une branche supprimée utilise aussi `git.Push`. Le test CONT-008 a déjà montré la limite de compatibilité de la création sous le contrat expected-base étudié : refus API 500 et message Web générique lorsque le contexte de base attendu manque.

La suppression API/Web appelle `services/repository/branch.go::DeleteBranch`. Le service vérifie si la branche est actuellement protégée, récupère son commit, inscrit la suppression et exécute `git branch -D` dans une transaction de base. Ce n’est pas un push receive-pack avec pré-receive, et l’appel ne transporte pas de compare-and-swap attendu de la ref. CONT-008/POC-061 ont déjà observé la suppression API d’une branche avec un expected-base périmé accepté par la route étudiée; le POC distingue ce cas du refus de suppression d’une branche protégée dans POC-076.

Le renommage est un autre chemin direct : le service `RenameBranch` vérifie l’existence et la destination, puis appelle `git branch -m` dans le callback de transaction qui change aussi le nom DB; le modèle déplace ensuite la règle de protection avec le nom. Le code de service ne montre pas de contrôle de protection avant la mutation. La route API PATCH est enregistrée avec `reqRepoWriter`, puis appelle ce service; le POST Web de renommage passe par un contrôle `CanCreateBranch` dans un groupe de réglages plus restrictif. Le comportement API n’est pas une simple hypothèse statique : POC-076 l’a testé sur le même commit amont avec un collaborateur non-admin, autorisé à merger mais non à pousser sur la branche par défaut protégée; PATCH a répondu 204, conservé le SHA sous le nouveau nom et déplacé la protection. La route Web de réglage a répondu 404 pour cette identité. POC-061 et CONT-008 montrent séparément que la politique de pre-receive ne voit pas ces mutations directes.

Ces résultats établissent que placer uniquement un token obligatoire dans le pre-receive ne borne pas l’ensemble des writers Forgejo. Une protection de branche, une liste blanche de mergeurs, un contrôle du client ou un proxy API ne suffit pas à démontrer que les helpers serveur internes ne peuvent pas déplacer ou supprimer le ref.

### État PR et écritures après le ref

Le chemin post-receive charge l’événement de push puis met à jour la base. Pour un push de merge PR, `handlePullRequestMerging` enregistre le commit de mise à jour et appelle `SetMerged` après le déplacement du ref. Les chemins `MergedManually` et d’autodétection manuelle peuvent aussi modifier l’état `HasMerged` sans effectuer eux-mêmes le push correspondant. Le statut DB et le ref Git ne constituent donc pas une transaction atomique; POC-002 a déjà démontré un faux marquage manuel avec un commit concurrent.

## Conséquence d’architecture

Nous gardons Forgejo comme socle Git et faisons de la couche agentique un service externe, sans adopter les changements expérimentaux du scratch. Mais le profil multi-writer Forgejo ne peut pas être présenté comme ayant une précondition serveur universelle sur la base des hooks seuls.

Deux options cohérentes restent à comparer : réduire et démontrer les writers disponibles à l’autorité de fusion gate-only, ou proposer à l’amont un mécanisme serveur de mutation commun. Ce dernier devrait lier le SHA courant de la ref, la version de policy/revue, l’identité autorisée et les effets de base de données/LFS au moment de l’écriture; les mutations directes de branche, actions, tags/wiki, imports/miroirs et les chemins internes devraient être inclus ou refusés. LFS nécessite un état durable mis en attente, promotion idempotente et reprise sur crash sous la même décision de ref; une lecture préalable de reachability ne fournit pas cette clôture. Aucun chiffrage fiable en heures upstream ne découle du diff agrégé de POC antérieurs.

## Vérifications, sources et limites

Commandes : `git -C /tmp/forgejo-v17-base-cas-poc rev-parse HEAD`; `git describe --tags --exact-match HEAD`; `git status --short`; `git diff --shortstat HEAD`; lectures `git show HEAD:<path>` et recherches `git grep ... HEAD`. HEAD correspond au tag v17.0.0 exact. Le diff global du scratch est de 20 fichiers suivis, +1709/−85 lignes, plus 43 éléments non suivis; il agrège plusieurs POC et ne représente pas le coût d’un patch.

Lectures principales : `routers/api/v1/api.go:895-901,1092-1102`; `routers/api/v1/repo/file.go:427-432,549-649,652-802,834-948`; `routers/api/v1/repo/branch.go:91-167,170-287,431-460`; `routers/web/web.go:1487-1519`; `routers/web/repo/editor.go:265-344,505-563,677-778`; `routers/web/repo/branch.go:88-166`; `services/repository/files/update.go:75-150,201-228,249-268,291-325,465-505`; `services/repository/files/upload.go:54-165`; `services/repository/files/patch.go:99-210`; `services/repository/files/temp_repo.go:300-323`; `services/repository/branch.go:342-427,435-499`; `models/git/branch.go:309-366`; `modules/git/repo_branch.go:100-118,154-158`; `routers/private/hook_pre_receive.go`; `routers/private/hook_post_receive.go`; `services/pull/merge.go`; `services/pull/check.go`; `services/pull/merge_manual.go`; `services/automerge/automerge.go`.

Preuves antérieures associées : [POC-076](../forgejo-v17-merge-whitelist-rename-poc-076/local-run.md), [POC-061](../forgejo-v17-cas-route-lfs-stage-poc-061/local-run.md), [CONT-008](../forgejo-v17-writer-fence-lfs-recovery-008/local-run.md), [CONT-315](../continuation-315-forgejo-merge-check-review-001/local-run.md), [CONT-319](../continuation-319-forgejo-merge-mode-source-audit-001/local-run.md), [CONT-321](../continuation-321-forgejo-pre-receive-admin-bypass-boundary-001/local-run.md), [CONT-322](../continuation-322-forgejo-pre-receive-dismissal-race-001/local-run.md) et [CONT-323](../continuation-323-forgejo-web-pre-receive-dismissal-race-001/local-run.md). Aucun de ces POC scratch n’est adopté.

La revue reste ciblée et source-only dans ce lot : imports/migrations, miroir/synchronisation, tags/wiki, runners Actions, suppression/restauration de dépôts, tous types de refs, stockage PostgreSQL et tous les hooks tiers ne sont pas intégralement parcourus ici. Les noms et permission groups des routes sont lus dans le code; seule la combinaison de renommage API de POC-076 et les autres scénarios explicitement cités disposent de preuves runtime antérieures. Aucun test nouveau ni mesure d’impact n’a été exécuté. La séparation réseau MISSION-012 reste une preuve locale Linux/aarch64 sous local-test-adapter, pas une politique Forgejo universelle.

## Prochaine action

Ne pas répéter les essais de branche déjà prouvés. Construire, dans un checkout jetable propre de v17.0.0, les cas restants qui changeraient la décision : hook actif pendant une modification concurrente d’approbation/base sur chemins API/Web/Actions autorisés; mutations directes par un rôle gate-only; écritures upload/patch via LFS au refus, interruption et reprise, avec contrôle exact des refs, métadonnées et objets. Séparer les patchs de test expérimentaux afin d’estimer le plus petit changement amont et la compatibilité par surface. En parallèle, transférer le travail à GitLab PC4 dès que le port/end-point fourni par l’utilisateur est joignable; jusque-là aucune ref distante n’a été lue et aucun octet n’a été envoyé.
