# Audit des points d’intégration LFS — Forgejo v17.0.0

Date : 2026-10-24  
Dépôt scratch : `/tmp/forgejo-v17-base-cas-poc`  
Révision officielle : `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`  
Licence source : GPL-3.0-or-later, d’après le dépôt officiel déjà vérifié dans l’inventaire.

## Portée et provenance

Lecture ciblée du contrat `storage.ObjectStorage`, de `lfs.ContentStore`, des références `storage.LFS` et `NewContentStore`, des URL, suppressions, doctor/GC, dump/migration, et writers/readers trouvés par `rg` dans les fichiers Go non tests. Le détail reproductible, les lignes et les empreintes des sources scratch sont dans [callsite-inventory.log](callsite-inventory.log).

Le HEAD du scratch est le commit officiel indiqué, mais son worktree conserve des POC antérieurs. `services/lfs/server.go`, `services/repository/delete.go` et `models/git/lfs.go` portent notamment des changements expérimentaux déjà présents avant ce lot. Les références ci-dessous sont celles des sources présentes dans le scratch au moment de l’audit; le test POC-025 n’a modifié aucun fichier de production Forgejo.

Cet audit est une carte statique ciblée, pas une analyse AST exhaustive ni une validation de chaque backend. La recherche des appels `lfs.NewContentStore`/`lfs_module.NewContentStore` dénombre 11 constructions externes de `ContentStore` dans huit fichiers de production, en excluant l’appel interne de `ReadMetaObject`; neuf références directes à `storage.LFS` apparaissent dans neuf fichiers. Les appels du ContentStore des packages sont hors de ce décompte. Les wrappers indirects, SQL de métadonnées, migrations hors Go et plugins peuvent ajouter des chemins.

## Constat par seam

| Seam | Référence v17/scratch | Effet sur un stockage par génération |
|---|---|---|
| Contrat commun | `modules/storage/storage.go::ObjectStorage` (`Open`, `Save`, `Stat`, `Delete`, `URL`, `IterateObjects`; chaque opération reçoit un chemin) | Aucun jeton de génération, `operation_id`, delete conditionnel, inventaire des références actives ou résolution typée. L’interface sert aussi aux autres catégories de stockage; la changer globalement augmenterait le rayon de compatibilité. |
| Publication / nettoyage d’échec | `modules/lfs/content_store.go::ContentStore.Put` (Save sous `Pointer.RelativePath()`, puis Delete du même chemin si taille/hash/lecture échoue) | Une implémentation ne connaît pas la clé exacte du stage ni la génération ciblée par le cleanup. Le chemin OID logique ne peut pas sécuriser une suppression retardée. |
| Upload et writers | `services/lfs/server.go::UploadHandler`; `services/repository/files/upload.go::uploadToLFSContentStore`; `services/repository/files/update.go::CreateOrUpdateFile`; `modules/repository/repo.go::StoreMissingLfsObjectsInRepository`; `services/pull/lfs.go::createLFSMetaObjectsFromCatFileBatch` | Les écritures/associations ne partagent pas un contrat d’opération unique. Les chemins upload/API, import/pull et PR doivent tous publier ou résoudre le même mapping durable. |
| Lecture / existence / vérification | `ContentStore.Get/Exists/Verify`; lecteurs dans `services/mirror/mirror_push.go`, `services/pull/lfs.go`, `routers/web/repo/setting/lfs.go` et handlers LFS | `Open`/`Stat` résolvent seulement un chemin logique; les lecteurs ne reçoivent pas une génération qu’ils pourraient vérifier à l’ouverture. |
| URL signées | `services/lfs/server.go::buildObjectResponse`; `routers/web/repo/download.go`; `routers/api/v1/repo/file.go` | Ces appels contournent le `ContentStore`. Une URL signée doit pointer vers la génération exacte et conserver celle-ci au moins pendant sa durée de validité; aucun TTL/rétention de ce type n’est vérifié ici. |
| Suppression web | `routers/web/repo/setting/lfs.go::LFSDelete` | Supprime l’association en base, puis appelle `storage.LFS.Delete(oidPath)` quand le compte vaut zéro. Le commentaire upstream signale explicitement la course de stockage. La méthode ne porte pas de génération attendue. |
| GC dépôt/global | `services/repository/lfs.go::GarbageCollectLFSMetaObjects`; `services/repository/delete.go::DeleteRepository`; `models/system/notice.go::RemoveStorageWithNotice` | Le GC LFS passe par `ContentStore.Delete`; la suppression de dépôt compte les références en transaction puis efface physiquement après celle-ci via l’interface générique. La vérification DB et le delete ne sont pas une opération atomique/fencée. |
| Doctor et auto-fix | `services/doctor/storage.go::CheckStorage` / `commonCheckStorage` | Le checker transforme le chemin physique en OID par `strings.ReplaceAll(path, "/", "")`, puis supprime le chemin d’itération si la ligne OID n’existe pas. Pour `generation/<oid>/<gen>/<op>`, cet OID est différent; avec `autofix=true`, une clé d’objet référencée serait classée orpheline. POC-025 le vérifie sur SQLite + LocalStorage. |
| Dump / restauration | `cmd/dump.go::dumpRepos` | L’archive écrit le chemin retourné par `IterateObjects` dans `data/lfs/<objPath>`. Une clé physique de génération change la représentation sauvegardée; aucun readback de restauration de mapping n’a été exercé. |
| Migration de stockage | `cmd/migrate_storage.go::migrateLFS` | Itère les lignes actives et copie source/destination avec `RelativePath()`; les deux côtés doivent conserver ou traduire le mapping, et les stages/pending doivent avoir une politique de migration explicite. |

## Diagnostic d’architecture

Le point de départ « Forgejo peu modifié + couche agentique par interfaces » reste viable pour la forge produit, mais un mapping LFS générationnel ne peut pas être introduit comme simple décorateur de `ObjectStorage`. Pour ce POC, garder le code Forgejo dans le scratch et définir une interface LFS spécialisée qui distingue explicitement :

1. réservation/idempotency key d’opération et upload vers une clé immuable;
2. vérification de contenu avant promotion transactionnelle sous précondition;
3. `Resolve` puis `Open` liés au jeton de génération retourné;
4. URL signée liée à cette même génération et rétention jusqu’à expiration;
5. suppression conditionnelle par génération attendue après recomptage/coordination;
6. inventaire/GC logique qui renvoie OID, génération, état et clé, sans demander à Doctor d’inférer un OID d’un chemin physique;
7. dump/migration/restore qui préservent le mapping actif et les stages reprenables.

Le contrat spécialisé doit être câblé dans tous les chemins de serveur concernés; les API du stockage générique pour avatars, packages et pièces jointes ne doivent pas recevoir implicitement une sémantique LFS. Les changements recensés touchent au moins 15 fichiers de production/points d’intégration avant tests et migrations : ce décompte de fichiers est une borne de portée, pas une estimation en jours. La compatibilité externe des API LFS, des scripts de dump/migration et les dialectes DB restent à mesurer. Aucun fork n’est recommandé sur la base de ce seul audit.

## Test associé

Le [POC-025](../forgejo-v17-lfs-doctor-generation-poc-025/local-run.md) appelle le vrai `doctor.CheckStorage` sur le harnais d’intégration SQLite et LocalStorage. La variante qui attend la conservation de la clé échoue; le test de diagnostic confirme ensuite que le doctor supprime la clé physique générationnelle alors que la ligne active OID reste en base. Le cas suppose la forme de clé envisagée; il ne prouve pas qu’un adaptateur générationnel soit déjà installé.

## Limites

Pas de test MinIO/S3, d’URL signée, de transfert Git LFS réel, de migration/restauration, d’interruption du processus, de GC concurrent, de fencing multi-processus, ni d’évaluation du coût de rebase upstream. Aucun code Forgejo n’a été copié dans le produit ou publié.
