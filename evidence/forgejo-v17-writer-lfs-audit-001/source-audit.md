# Audit source — writers Forgejo et publication LFS sous précondition

Date : 26 octobre 2026. Source officielle examinée : Forgejo v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Checkout : `/tmp/forgejo-v17-base-cas-poc`, détaché sur ce commit mais fortement modifié par un POC expérimental. Il n’est pas le code produit. Hachages des fichiers locaux et identifiants blob du commit de base : [source-hashes.txt](source-hashes.txt) et le contrôle `sha256sum -c` [source-content-SHA256SUMS](source-content-SHA256SUMS). Les sorties des recherches et le refus d’exécution Go sont conservés dans [source-inspection-output.txt](source-inspection-output.txt).

## Conclusion

Le champ expérimental `RequireExpectedBaseCommitID` ne crée pas une précondition serveur universelle. Le contrôle se trouve dans le hook `preReceiveBranch` (`routers/private/hook_pre_receive.go:255–262, 499–527`) et ne reconnaît que des pushes avec contexte serveur PR et SHA attendu. Les tags suivent `preReceiveTag` sans ce contrôle. Le POC ne rend pas non plus cette option administrable dans le DTO public observé : les essais la règlent en base.

Plusieurs writers ne passent pas par ce hook ou ne lui fournissent pas le contrat attendu :

| Writer | Source observée | Conséquence ou lacune |
|---|---|---|
| Push Git d’une branche | `preReceiveBranch` appelle le garde; `preReceiveTag` ne l’appelle pas. | Le garde peut refuser une branche sans contexte serveur, mais n’établit pas la couverture des tags ni de tous les writes internes. |
| Merge et rebase PR | `services/pull/merge.go:352–404` et `services/pull/update_rebase.go:42–103` transmettent une base attendue au push. AutoMerge appelle le même service merge (`services/automerge/automerge.go:231`). | C’est le chemin couvert par le hook. Les tests d’AutoMerge/rebase avec cette politique activée n’ont pas été exécutés dans cet audit. La suppression de branche après merge est un writer distinct. |
| Fichiers via API | `services/repository/files/update.go:294–324` prépare les objets LFS en attente mais appelle `t.Push` sans installer le contexte/SHA attendu. | Sous la règle du POC, ce chemin ne possède pas la preuve serveur exigée; le fichier API peut devenir incompatible au lieu d’offrir une mutation CAS utilisable. |
| Création/restauration | `services/repository/branch.go:342–365` pousse une branche sans contexte attendu; la restauration Web fait aussi un push. | Aucun jeton de base n’est transmis. Les tests POC correspondants sont du code de test, non exécutés ici. |
| Renommage/suppression de branche | `RenameBranch` appelle `gitRepo.RenameBranch` directement (`services/repository/branch.go:367–427`); `DeleteBranch` lit d’abord le commit puis appelle `gitRepo.DeleteBranch` dans une transaction DB (`:435–498`). | Ces mutations de ref ne traversent pas le garde `pre-receive`; suppression ouvre aussi une fenêtre entre lecture et mutation. Un contrôle dans le seul hook ne suffit pas. |
| Merge manuel | `services/pull/merge_manual.go:35–38` refuse de marquer la PR manuellement fusionnée quand la règle est active; l’auto-détection est désactivée (`:103–107`). | Refus ciblé, sans preuve d’une mutation de ref contrôlée ni d’une politique uniforme des handlers API/Web. |
| Upload Web LFS | `services/repository/files/upload.go:131–163` crée des associations LFS actives et stocke les objets avant `t.Push`. | Un refus du push peut laisser une association active pour du contenu qui n’est pas joignable par la branche. Ce chemin ne suit pas le staging en attente du fichier API. |
| Push interne | `cmd/hook.go:175–178, 304–317` ignore les hooks avec `GITEA_INTERNAL_PUSH=true`; la recherche des appels hors tests examinée n’a trouvé que l’initialisation de dépôt. | Exception actuellement étroite dans les appels observés, mais elle contourne les hooks par conception; elle doit rester explicitement bornée. |

## Publication et récupération LFS

Le merge construit une clé d’opération et stage les associations avant le push (`services/pull/merge.go:360–378`), transmet la base attendue au hook, puis promeut après succès Git (`:394–404, 437–440`). Si la promotion échoue après acceptation du ref, le serveur retourne une erreur alors que le merge existe et que l’opération LFS est pending.

Le chemin API de mise à jour écrit et stage l’objet, pousse le commit, puis vérifie la promotion (`services/repository/files/update.go:294–331`). L’handler convertit `LFSPublicationPendingError` en HTTP 500 (`routers/api/v1/repo/file.go:788–795`). Le code d’erreur porte une opération, mais l’audit de ces sources n’a pas identifié de route de consultation ni de clé idempotente client garantissant qu’un retry ne produira pas un deuxième commit. La sémantique de l’accusé doit distinguer « accepté une fois » de « encore présent sur la ref au moment du readback ».

Le POC ajoute des helpers paginés/reprenables dans `services/pull/lfs_recovery.go`; une recherche source exhaustive des appels de ces helpers hors tests ne trouve que leurs définitions. Le helper n’est donc pas un worker ni une reprise au démarrage. Les opérations non joignables restent différées sans politique de rétention constatée. `routers/private/hook_post_receive.go` vérifie et promeut après la mutation, mais le readback de ref et la promotion DB ne sont pas une transaction atomique.

## Type d’évidence et coûts visibles

C’est un audit statique ciblé du code courant et du commit Git épinglé, avec une lecture des signatures/chemins ci-dessus. Le code expérimental contient des tests d’intégration taggés et des logs historiques, mais leurs résultats ne sont pas attribués ici à l’état de source actuel. Aucun test, compilation, serveur ou migration n’a été exécuté dans cette tranche : la commande `go version` échoue actuellement (`go: command not found`) et aucun binaire n’a été trouvé dans les emplacements locaux vérifiés. Le POC n’est donc pas déclaré fonctionnel au-delà des preuves historiques séparément archivées.

Le diff courant du scratch compte 20 fichiers suivis modifiés, 1 709 insertions et 85 suppressions, plus des fichiers POC non suivis (tests, migration et récupération). C’est une mesure de surface de ce prototype, pas une estimation d’heures. Il touche hooks, chemins Git/API/Web, merge/rebase, stockage et modèle LFS. Aucun rebase sur une version Forgejo ultérieure ni coût annuel de maintien upstream n’a été mesuré. Ces écarts et la compatibilité manquante excluent l’adoption du patch pour le moment.

Aucun code Forgejo n’a été copié au produit. Le POC reste sous `/tmp`; aucune instance production, publication ou donnée privée n’a été utilisée.

## Prochaine preuve utile

Quand un Go toolchain local sera disponible, exécuter une matrice d’intégration séparée sur push Git branche/tag, API fichiers, upload Web LFS, création/restauration/renommage/suppression, merge/rebase/AutoMerge, merge manuel et push interne. Pour chaque writer, comparer base attendue valide/périmée/absente, SHA/ref avant-après, métadonnées LFS actives/pending, code HTTP et readback après interruption.

Pour LFS, ajouter avant adoption un statut consultable et une clé de requête idempotente, brancher une récupération bornée dans un worker ou au démarrage réel, puis tester un arrêt du processus Forgejo après mise à jour Git et avant promotion DB. Une primitive CAS ne peut être évaluée qu’après sa couverture de tous ces writers; le POC actuel ne la démontre pas.
