# CONTINUATION-327 — une mutation API Forgejo passe sous le verrou du gate

Date : 27 octobre 2026, 23:30 America/Montreal (2026-10-28 03:30 UTC). Worktree produit : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Scratch Forgejo : `/tmp/forgejo-v17-base-cas-poc`, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571` (v17.0.0 officiel). Ce scratch est déjà fortement modifié par des POC CAS/LFS; aucun patch Forgejo n’est adopté.

## Question et montage

CONT-326/POC-076 avaient établi séparément que le PATCH API `UpdateBranch` peut renommer une branche protégée pour un collaborateur non-admin qui peut merger mais ne peut pas pousser. Ici le test distinct `TestPOCGateBaseLockDoesNotFenceProtectedBranchRenameAPI` établit l’interaction avec le verrou local du produit.

Un petit exécutable Rust, compilé hors du dépôt Forgejo contre le worktree produit courant, appelle le vrai `BaseBranchScope::new` et `BaseBranchMergeLock::acquire`. Il tient le verrou dans un processus séparé et écrit un fichier prêt seulement après l’acquisition. Le test Forgejo attend ce fichier avant d’envoyer le PATCH; le processus ne libère le verrou qu’après le readback.

La fixture SQLite Forgejo v17 crée `forge-lock-merger`, non-admin, collaborateur `write`, unique membre de la merge whitelist d’une règle sur la branche par défaut `master`, avec push désactivé. Le readback confirme `Protected=true`, `UserCanMerge=true`, `UserCanPush=false`. Le paramètre `RequireExpectedBaseCommitID` est activé. La portée verrouillée est l’origine de l’instance de test, `user2/repo1`, branche `master`.

## Résultat observé

Pendant que le verrou exact du gate demeure acquis par le processus Rust distinct, le vrai handler Forgejo reçoit `PATCH /api/v1/repos/user2/repo1/branches/master` et répond **204**. Avant de libérer le verrou, le readback observe : le nouveau nom `locked-api-rename-target` pointe sur le même OID qu’avant; l’ancien nom `master` a disparu; la règle de protection existe sous le nouveau nom.

La commande Go ciblée termine avec code 0 : `--- PASS`, `PASS`, `ok forgejo.org/tests/integration 1.575s`. C’est une observation déterministe à barrière (`n=1` final), pas une mesure statistique. Le test vérifie une voie d’écriture directe; il n’effectue pas de merge.

## Interprétation

`BaseBranchMergeLock` coordonne les processus qui prennent le même `flock`; l’API Forgejo ne lit pas ce fichier et n’honore pas le verrou. L’attestation d’un verrou local ne prouve donc pas que les writers Forgejo hors gate sont exclus. Les probes réseau de CONT-324 concernent le processus agent isolé et ses routes; elles ne bloquent ni ne contrôlent automatiquement l’API accessible au gate ou à d’autres acteurs.

Cette observation renforce la réserve de CONT-324 : `external_writers_excluded=false` dans le parcours MISSION-012 local-test-adapter signifie que ce parcours est une preuve de composition en bac à sable, pas une autorisation de déploiement ou une clôture de concurrence pour un dépôt partagé. Un mode de fusion sûr doit démontrer une exclusion positive de ces routes/writers, ou reposer sur un contrôle serveur commun, atomique et récupérable; le verrou local seul ne suffit pas. Aucun fork ou changement de Forgejo n’est adopté.

## Couverture et limites

- L’instance est le harnais d’intégration Forgejo v17 en SQLite, lancé par `onApplicationRun`, et non un binaire/deploiement intact. Le scratch contient d’autres POC sales.
- Les blobs de `routers/api/v1/api.go`, `routers/api/v1/repo/branch.go`, `services/repository/branch.go`, `models/git/branch.go`, `models/git/protected_branch.go` et `modules/testlogger/testlogger.go` sont identiques à ceux du HEAD upstream; les identifiants de blobs et SHA-256 sont dans `source-hashes.txt`.
- Un seul rôle, une instance SQLite, une branche et le PATCH de renommage sont couverts. Ce test ne démontre pas un merge concurrent, toutes les routes, SSH, workers, PostgreSQL, cluster, tags/wiki/imports, ni les autres writers.
- Forgejo journalise pendant l’arrêt des tâches de fond avec `context canceled`. Le testlogger affiche `FATAL ERROR` mais la source v17 indique qu’il n’appelle actuellement pas `t.Errorf` pour ce cas (commentaire « too flacky »); le test finit bien avec code 0. Ces lignes sont gardées dans le journal.
- Deux premières erreurs de montage et un blocage de nettoyage du harnais sont consignés dans `harness-notes.txt`; elles ne correspondaient pas à un refus Forgejo. La dernière exécution a été faite après correction du harnais.
- Aucune source Forgejo n’est copiée dans le produit; le helper et le test POC restent dans `/tmp/forgejo-v17-base-cas-poc`. Aucun secret distant, modèle payant, publication ou instance de production n’a été utilisé.

Commandes, outil, hashes, résultat structuré et journal brut sont joints à ce reçu.
