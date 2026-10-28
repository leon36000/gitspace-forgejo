# Forgejo v17 — course après lecture du ref et avant publication LFS — POC-074

Date : 25 octobre 2026. Le scratch `/tmp/forgejo-v17-base-cas-poc` part de Forgejo `v17.0.0`, commit amont `0d791421f85ca13a63b665053d81ea765fbbc571`, et contient de nombreux changements expérimentaux antérieurs. Ce lot ajoute une seam de test et un test d’intégration dans le scratch seulement; aucun code Forgejo n’est copié dans le dépôt produit.

## Question et observation

La lecture de `refs/heads/master` dans `routers/private/hook_post_receive.go::HookPostReceive` et la promotion SQL par `git_model.PromoteLFSPendingMetaObjects` sont-elles protégées par un fence commun ?

`TestPOCGenericFileLFSAPI201CanFollowAStaleHookObservation` appelle la vraie route `POST /api/v1/repos/.../contents/...`, avec les hooks pre-receive et post-receive réels de l’application d’intégration. Une seam dédiée déplace la ref de la branche vers son commit de base **après** que le hook a observé le commit candidat et **avant** l’insertion de l’association LFS active. La route répond `201 Created`; la ref courante reste à la base, le SHA de la réponse est différent, l’association LFS est active et il ne reste aucune ligne pending. Le log donne `candidate=680796da2f670e0bedb48e4c14fcc04402f384b1`, `current_ref=8290084d8d95a6bf778a5f82ffc9c04ce49da678`, `api_status=201`, `active_lfs=true`.

Ce résultat démontre que le contrôle de ref de ce patch scratch n’est pas atomique avec la promotion SQLite. Le test est une caractérisation de la fenêtre de course, pas un succès de sécurité; la mutation concurrente est déterministe et simulée par `git update-ref` à la seam, sans deuxième processus Git ou instance Forgejo.

## Vérification

Le run final exécute les cinq scénarios d’acquittement/récupération du lot, dont ce cas, sous le build tag POC et SQLite : **5 passés, 0 échec**. Ce statut signifie que les scénarios de caractérisation et de reprise satisfont leurs assertions; il ne rend pas acceptable le `201` observé pendant la course. Le test de panne SQLite émet les logs d’erreur injectés attendus tout en terminant avec succès.

`gofmt -d` ne produit aucune différence; `git diff --check` est vide. La commande, le journal complet, le commit de base et les empreintes sont conservés dans ce dossier. Environnement : Go 1.27.3 linux/arm64, Git 2.43.0, SQLite, `GOPROXY=off`, aucun téléchargement de module.

## Portée et décision

La promotion LFS pending après validation du ref ferme le défaut de promotion avant vérification, mais pas la fenêtre lecture Git → commit SQL. Une seconde lecture sans exclusion des writers laisserait encore une course. Un verrou mutex du processus ne couvre ni les autres processus Forgejo ni les writers directs; un verrou Git de ref prend fin avant la promotion post-receive; un verrou DB ne bloque pas les mutations Git qui ne le prennent pas.

Le balayage source ciblé a confirmé plusieurs familles d’écrivains : les pushes Git/API fichier passent par `TemporaryUploadRepository.Push` et les hooks; `routers/api/v1/repo/branch.go::DeleteBranch` et les handlers Web appellent directement `services/repository/branch.go::DeleteBranch`; `RenameBranch` modifie aussi les refs hors de ce hook; les services de merge utilisent des pushes et des mutations `update-ref`; des chemins d’import/migration, miroir, wiki et package ont leurs propres writers. `services/pull/pull.go::UpdateRef` met aussi à jour directement des refs de PR. Cette recherche orientée call sites est bornée, pas un audit exhaustif des plugins ou de toute écriture indirecte.

**Compatibilité et maintenance :** une précondition globale exigerait de faire passer toutes les mutations de ref par un service/fence partagé, d’ajouter une base attendue aux contrats API/Web sans champ aujourd’hui, de définir le refus des clients Git et des merges automatisés, puis de gérer reprise et expiration à travers pre-receive/post-receive et les processus distribués. La source examinée contient déjà des bypass API/Web (POC-061/070) et un chemin API de création refusé de façon opaque (POC-062). Il s’agit d’au moins plusieurs sous-systèmes Forgejo et d’une adaptation à maintenir à chaque évolution amont; aucune estimation en heures fiable n’est disponible. Ne pas adopter le patch de hook ni le présenter comme une garantie universelle.

Un balayage lexical large des chemins `routers/`, `services/` et `modules/` fait ressortir **22 fichiers candidats**. Ce total inclut des refs de PR, une copie de staging, une exportation de migration et des fonctions `fetch`; il ne signifie donc pas 22 writers confirmés de la branche cible. L’inventaire conservé sépare ces catégories dans `writer-scan.txt`. Les incompatibilités déjà observées touchent au moins cinq opérations/points d’entrée : API delete/rename acceptés malgré la base périmée, renommage/suppression Web sans transmission de base, création API de branche refusée sous forme 500, et écriture de fichier refusée par le hook strict. Ces cas sont des tests POC antérieurs, pas une estimation complète du nombre de clients affectés.

Limites : test scratch v17 fortement modifié; SQLite et un hôte seulement; seam déterministe; pas de PostgreSQL, cluster, crash au milieu d’une mutation, benchmark ni instance de production. Le résultat n’établit pas que Forgejo amont vierge possède ce défaut : il réfute l’atomicité du patch LFS expérimental. La prochaine décision est de comparer une autorité unique avec contrôle de permissions à un service de mutation de ref maintenu, puis d’évaluer un fence durable multi-processus sur plusieurs chemins représentatifs avant toute adoption.
