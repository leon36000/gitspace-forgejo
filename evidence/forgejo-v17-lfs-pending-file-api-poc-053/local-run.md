# Forgejo v17 — publication LFS différée du writer générique de fichiers

Date : 25 octobre 2026. Source officielle auditée : Forgejo `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Expérience menée dans le scratch préexistant `/tmp/forgejo-v17-base-cas-poc`, dont le worktree était déjà fortement modifié. Aucun code Forgejo n’a été copié dans le produit ni déployé.

## Changement expérimental

Dans `services/repository/files::ChangeRepoFiles`, les octets LFS vont toujours dans le content store avant le push, mais la métadonnée d’association reste pending et identifie le dépôt, la ref cible, la base préparée et le commit candidat. Le post-receive du writer générique relit la ref et ne promeut que si elle pointe encore vers le candidat. Si cette étape manque, `RecoverLFSPendingMetaObjects` ne promeut que lorsque le candidat est atteignable depuis la ref cible. Un seam de test permet d’omettre la promotion à cet endroit.

Le delta de cette tranche touche deux fichiers de production scratch (`services/repository/files/update.go`, `routers/private/hook_post_receive.go`) et des tests d’intégration dans `tests/integration/git_push_test.go`; la signature du helper interne `CreateOrUpdateFile` reçoit aussi la liste des objets LFS à stage. Le scratch avait d’autres changements antérieurs, conservés en place. Ce décompte décrit uniquement les surfaces ciblées par cette expérience, pas le coût total d’un fork.

## Résultats observés

La commande hors ligne utilise Go `go1.27.3 linux/arm64`, `GOTOOLCHAIN=local`, `GOPROXY=off`, le tag `sqlite` et le serveur d’intégration local. Les deux tests passent :

- `TestExpectedBasePolicyRejectsFileAPIAndLeavesLFSMetadata` : le writer API reçoit `403`, la ref demeure inchangée, aucune association LFS active n’existe, une ligne pending reste enregistrée et la lecture Git LFS retourne `404`. Cela corrige le résidu actif et lisible démontré par POC-002; la ligne pending confirme qu’un nettoyage borné reste requis.
- `TestPOCGenericFileLFSRecoversAfterPostReceiveInterruption` : après la création d’un fichier LFS, le seam omet la promotion alors que la ref a avancé. L’association reste invisible et pending; la récupération ciblée promeut une opération, puis le GET LFS rend les octets attendus. Une seconde écriture sans le seam vérifie que le post-receive promeut immédiatement et laisse zéro ligne pending.

Sortie brute : [scratch-integration-test.log](scratch-integration-test.log). La commande exacte est :

```sh
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -count=1 -v -timeout=20m -tags sqlite ./tests/integration \
  -run '^(TestExpectedBasePolicyRejectsFileAPIAndLeavesLFSMetadata|TestPOCGenericFileLFSRecoversAfterPostReceiveInterruption)$'
```

`gofmt -d` ne produit aucune différence et `git diff --check` réussit sur les trois fichiers ciblés. Voir [tool-versions.txt](tool-versions.txt), [scratch-revision.txt](scratch-revision.txt), [source-hashes.txt](source-hashes.txt) et [format-and-diff-checks.txt](format-and-diff-checks.txt) pour l’identité des sources et des outils.

## Limites et compatibilité

Le seam omet une étape de hook; ce test ne tue pas un processus et ne démontre pas un SIGKILL réel. Le run couvre une fixture Forgejo d’intégration avec SQLite et stockage local, une seule exécution; il ne teste pas une course de writers entre la lecture de ref et la promotion DB. Cette lecture puis promotion n’est pas atomique avec Git. Le récupérateur reste non paginé, sans budget par passage, sans caller de production ni politique de rétention/nettoyage. Le chemin d’upload générique `UploadRepoFiles`, les uploads du protocole Git LFS, les miroirs, imports et autres writers n’ont pas été changés. Cette adaptation ne rend donc pas la précondition CAS globale.

Pour la route testée, les clients gardent le contrat HTTP existant : une écriture acceptée expose le LFS après confirmation de ref; une écriture refusée ne publie plus une association active. L’adoption upstream demanderait encore la pagination et la rétention du pending, un worker de reprise gouverné, les autres call sites LFS et un arbitrage des writers sans précondition. Aucun coût en heures n’est avancé : la tranche touche deux surfaces de production scratch, mais l’audit des routes 052 montre que l’effort upstream total est plus large. Le choix d’interface autour de Forgejo reste préférable à une adoption prématurée du fork tant qu’aucune capacité essentielle ne dépend de ces changements internes.

Le parcours LLM, l’instance de production et la comparaison contrôlée avec Forgejo + agent + CI conventionnels ne sont pas concernés ni démontrés par ce POC.
