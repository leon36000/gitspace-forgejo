# Forgejo v17 — limite du CAS de base et récupération LFS en attente (043)

Date : 25 octobre 2026. Examen local du dépôt officiel Forgejo v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, sous licence GPL-3.0-or-later. Le checkout scratch `/tmp/forgejo-v17-base-cas-poc` a le même HEAD mais reste fortement modifié (18 chemins suivis modifiés/ajoutés, 28 non suivis). Aucun code de ce scratch n’est copié au dépôt produit ni soumis; le `CONTRIBUTING.md` du projet exclut les contributions écrites par des agents de code.

## Précondition côté serveur : ce que le patch couvre

La branche scratch ajoute `PullRequestsConfig.RequireExpectedBaseCommitID` dans `models/repo/repo_unit.go`; le défaut est `false`, et aucune interface de configuration d’administrateur ou route REST de réglage n’a été trouvée dans les références de cette propriété. `routers/private/hook_pre_receive.go::assertExpectedBaseCommitID` s’exécute sur l’écriture de branche et, si le réglage est vrai, refuse un contexte absent, un token vide ou une valeur différente du `oldCommitID` reçu. Les champs de hook sont lus dans l’environnement du serveur par `cmd/hook.go`; le chemin interne de merge les remplace explicitement via `modules/repository/env.go::WithExpectedBaseCommitID`.

La voie `services/pull/merge.go::doMergeAndPush` calcule cependant `mergeBaseSHA` depuis son clone de travail au moment de l’opération, puis l’envoie comme précondition. Cette comparaison protège l’intervalle entre le clone de merge et la mise à jour du ref; elle ne lie pas à elle seule ce SHA à la base sur laquelle les tests et la preuve indépendante ont été produits. C’est une inférence du flux de code, non un résultat d’essai : si la base avance avant le début de l’opération de merge, le clone peut relire la nouvelle base et présenter une précondition cohérente avec le ref, alors que la preuve reçue par l’autorité reste liée à l’ancienne base.

La limite ressort aussi du modèle AutoMerge officiel : `models/pull/automerge.go::AutoMerge` stocke le PR, auteur, style, message, suppression de branche et date, pas un SHA de base. `services/automerge/automerge.go::handlePullRequestAutoMerge` vérifie le SHA de tête et les checks de la PR, puis appelle le service de merge; il n’a pas de base de preuve persistée à comparer. La propriété stricte est vérifiée par un test heureux dans `tests/integration/pull_merge_test.go::TestPullMergeExpectedBaseCommitID`; ce test ne fait pas avancer la base après l’évaluation.

Une revue négative déjà archivée a exercé les routes API/Web et variantes AutoMerge/manually-merged lorsque le client omet le champ. Le run consolidé des quatre fonctions se trouve dans [POC-002](../forgejo-v17-base-cas-poc-002-lfs-residue/local-run.md); son journal SHA-256 est `17a524a582e71cea4aa7639b10df35d281b7e39690c3c905a9d58777665fbc8b` et son fichier source de tests a le SHA `6144e757036a6341e494e9ba7eca81f53d37b60a813afa22cd2e802e34501fec`. Ces résultats sont ceux de cet ancien snapshot, pas une nouvelle relance de ces routes durant 043. Le patch courant bloque le chemin manuel sous le flag strict mais ne stocke toujours pas la base au moment de la preuve dans AutoMerge.

### Compatibilité et coût de maintenance observés

Avec le réglage désactivé, les chemins existants ne sont pas censés être affectés. Une fois activé, le hook refuse les pushes directs sur la branche ciblée sans contexte serveur PR; un test scratch de l’API de création de fichier observe aussi HTTP 403 et un ref inchangé, mais l’association LFS en amont subsiste (résidu documenté dans POC-002). Le patch bloque la marque `manually-merged` sous le mode strict. Il faut donc une migration vers les routes de PR, une interface de réglage, et un contrat de refus clair pour les intégrations qui écrivent directement.

Pour satisfaire la précondition exigée par le produit, il reste à transporter le SHA de base lié à l’évaluation indépendante, à le persister dans les opérations AutoMerge et à le vérifier au moment de la fusion; définir aussi le comportement des vieux AutoMerge déjà en base. Cela traverse au minimum le modèle et le stockage AutoMerge, l’API et le Web, le worker d’autofusion, le service merge/rebase et le hook privé. L’estimation en heures et le coût de maintien face aux mises à jour upstream ne sont pas mesurés; le diff scratch courant mélange CAS, LFS et autres expériences, donc ses 18 fichiers suivis ne sont pas un chiffrage propre du seul CAS.

## Publication LFS staged puis promue

Le patch scratch définit `models/git/lfs.go::LFSPendingMetaObject`; `services/pull/merge.go` et `update_rebase.go` écrivent les associations en état pending avant le push Git, abandonnent leur stage si le push est refusé, et appellent `PromoteLFSPendingMetaObjects` après succès. Un crash entre la mise à jour du ref et cette promotion laisse l’association pending. Le helper `services/pull/lfs_recovery.go::RecoverLFSPendingMetaObjects` lit les lignes pending, exige que le commit candidat soit atteignable depuis le ref cible exact, promeut l’opération idempotemment et conserve les opérations ambiguës.

Le test `services/pull/lfs_recovery_test.go::TestRecoverLFSPendingMetaObjectsAfterInterruptedPostReceive` simule la fenêtre après ref update en effectuant un push direct sans post-receive, puis appelle manuellement le helper; il vérifie deferred avant que le ref cible existe, promotion après l’update, puis rejeu sans effet. Deux autres cas couvrent le candidat manquant et le ref semblable mais différent.

Commande exécutée avec le Go local `go1.27.3 linux/arm64`, sans réseau et sans changement de dépendances :

```sh
GOTOOLCHAIN=local GOPROXY=off GOFLAGS=-mod=readonly \
  /tmp/forgejo-v17-toolchain/go/bin/go test -tags sqlite ./services/pull \
  -run 'TestRecoverLFSPendingMetaObjects' -count=1
```

Résultat : paquet `forgejo.org/services/pull` réussi; les trois tests correspondants passent. Une première tentative sans `-tags sqlite` s’est arrêtée avant les tests, car le driver SQLite du harnais n’était pas enregistré; le log de ce diagnostic et la relance verte sont conservés ici. Le manifeste `rg -n 'RecoverLFSPendingMetaObjects|...' --glob '*.go' .` retrouve le helper et ses tests, sans appelant production dans le scratch. Ainsi, le protocole de récupération est testé quand on l’appelle, mais aucun démarrage, worker ou tâche durable ne l’appelle après interruption. Ce test n’est pas un crash réel du serveur Forgejo.

## Décision et limites

Le patch expérimental ne satisfait pas encore l’invariant « preuve et base testée identiques au ref fusionné » et la récupération LFS n’est pas opérationnelle tant qu’un appelant de reprise n’est pas raccordé. La direction sûre reste : autorité de fusion extérieure qui relit immédiatement les mêmes hashes, puis, si une garantie contre tous les writers Forgejo est requise, précondition server-side propagée depuis la preuve au travers de chaque route, avec AutoMerge liée à cette valeur. Pour LFS, brancher un worker/startup idempotent borné, conserver les pending jusqu’à preuve du ref exact, et définir rétention/nettoyage avant d’adopter le protocole.

Le scratch n’est pas un fork recommandé. Forgejo `CONTRIBUTING.md` n’accepte pas de code écrit par agents; les fichiers POC ne doivent pas être envoyés upstream. Aucun service de production, transfert privé ou appel payant n’a été utilisé. Le dernier run Go prouve seulement ces trois fonctions de récupération; API/Web/AutoMerge ne sont pas relancés dans cette tranche.
