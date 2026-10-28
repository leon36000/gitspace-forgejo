# FORGEJO-V17-LFS-API-ACK-ORDER-POC-069 — résultat local

Date : 25 octobre 2026. Le scratch `/tmp/forgejo-v17-base-cas-poc` est basé sur Forgejo officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, mais contient de nombreuses modifications de diagnostic. Le test n’est donc pas exécuté sur l’arbre amont intact. Le test ajouté est sous le build tag `forgejo_poc_lfs_ack_order`; aucun code Forgejo n’est copié dans le produit.

## Question vérifiée

Une écriture API de fichier LFS peut-elle recevoir HTTP 201 après que le ref Git a avancé alors que l’objet LFS reste invisible, si la promotion post-receive est interrompue ?

## Résultat observé

`TestPOCGenericFileLFSAPI201CanPrecedePromotion` démarre l’application d’intégration Forgejo avec SQLite local, ajoute une règle `.gitattributes`, puis appelle réellement la route HTTP `POST /api/v1/repos/.../contents/...`. Un seam explicitement réservé aux tests saute la promotion post-receive. Le serveur répond `201 Created`; le SHA du commit retourné est celui du ref cible. À cet instant, aucune association LFS active n’existe et le GET LFS répond `404`. Le test invoque ensuite le recovery existant : il promeut l’opération pending et le même GET répond `200` avec les octets attendus.

Le run ciblé passe : 1 test réussi, 0 échec, 0 ignoré, paquet `tests/integration` en 1,448 s. Toolchain : Go `1.27.3`, `linux/arm64`; `GOTOOLCHAIN=local`, `GOPROXY=off`. Le premier essai de compilation échouait sur un import inutilisé du nouveau test; il a été corrigé avant le run vert. Les deux sorties sont conservées dans ce dossier.

Le journal de test inclut le `log.Error` attendu lors du GET 404 et des `context canceled` au nettoyage de l’application. Le harnais imprime un agrégat « FATAL ERROR » pour ces messages, mais le test et le processus Go terminent explicitement `PASS`/code 0; ne pas interpréter cet agrégat comme un run sans signal de journalisation.

Commande du run final :

```sh
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 -v -timeout=20m -tags sqlite,forgejo_poc_lfs_ack_order ./tests/integration -run '^TestPOCGenericFileLFSAPI201CanPrecedePromotion$'
```

## Ordre de code pertinent

- Scratch `services/repository/files/update.go:266-284` : l’association est mise en attente après création du commit et avant `TemporaryUploadRepository.Push`.
- Scratch `routers/private/hook_post_receive.go:171-194` : le post-receive confirme la ref, puis promeut. Si la promotion échoue, le code journalise l’erreur mais ne renvoie pas d’erreur à ce point. Le seam du test saute cette promotion; il ne simule pas une panne de base de données.
- Scratch `routers/api/v1/repo/file.go:644-649` : la route retourne 201 si le service d’écriture retourne sans erreur.
- Le nouveau test du scratch est `tests/integration/lfs_api_ack_order_poc_test.go`; empreintes des trois fichiers modifiés du scratch et du test dans `source-sha256.txt`.

Une lecture distincte des autres writers indique que merge et rebase de PR font aussi leur préparation avant le push et retentent une promotion côté service après celui-ci. Leur échec peut donc produire une erreur de service après l’avancement de la ref. Les branches, renommages/suppressions et `GITEA_INTERNAL_PUSH=true` ne partagent pas toutes le même point de contrôle pre-receive; la politique expérimentale de base attendue ne constitue pas une précondition universelle de tous les writers. Cette cartographie de code n’est pas un test de panne injectée pour ces chemins.

## Interprétation, limite et garde-fou de conception

Le parcours intégré prouve une fenêtre de cohérence concrète : succès HTTP et commit publiés ne garantissent pas immédiatement la disponibilité de l’objet LFS lorsque la promotion est sautée. Il ne prouve pas qu’un processus Forgejo s’est réellement arrêté, qu’une erreur de stockage est survenue, que le recovery fonctionne comme worker automatique, ni que le comportement de toutes les API/PR est identique. Le seam est une interruption déterministe simulée. Dans le code scratch, une erreur réelle de promotion générique est seulement journalisée; ce comportement mérite un choix explicite de contrat d’API.

Avant tout nettoyeur automatique, conserver les associations `pending` en cas de doute. Une absence temporaire du candidat ne suffit pas à conclure au rejet : POC-068 a montré qu’un `Discard` fondé sur cette observation périmée peut retirer l’association après publication. Une transition terminale de nettoyage doit être autorisée uniquement par un résultat de rejet définitif et un fence partagé avec les writers de refs; le CAS SQL isolé ne clôt pas cette course. La suppression physique de blobs partagés demande encore une génération/fence distincte.

Il s’agit d’un diagnostic scratch, pas d’une proposition de fork adoptée. Les coûts de compatibilité et de maintenance d’un patch upstream, un parcours avec LLM, AgentOS, et une comparaison contrôlée restent non évalués.

## Fichiers de preuve

- `local-run.log` : sortie complète du run final.
- `initial-compile-failure.log` : premier essai, avant correction du test.
- `source-sha256.txt` : base commit et empreintes du code examiné dans le scratch.
- `commands.txt` : commandes de reproduction et limites.
- `SHA256SUMS` : intégrité locale du dossier.
