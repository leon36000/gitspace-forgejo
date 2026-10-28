# FORGEJO-V17-LFS-API-PROMOTION-BEFORE-ACK-POC-071

Date : 25 octobre 2026. Diagnostic dans le scratch `/tmp/forgejo-v17-base-cas-poc`, basé sur Forgejo officiel v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le checkout scratch est fortement modifié par des POC; ce résultat n'est pas un test de l'arbre amont intact. Aucun code Forgejo n'est copié dans le produit et aucun fork n'est adopté.

## Question

La route générique d'écriture de fichier peut-elle répondre HTTP 201 si le hook `post-receive` a installé le commit mais n'a pas promu l'association LFS? Le service peut-il retarder sa réponse jusqu'à la promotion, en laissant une erreur récupérable si celle-ci échoue après le push?

## Observation rouge

Le test d'intégration `tests/integration/lfs_api_ack_order_poc_test.go::TestPOCGenericFileLFSAPI201IsReturnedAfterPromotion` démarre l'application Forgejo de test avec SQLite, installe une règle `.gitattributes`, puis exerce réellement `POST /api/v1/repos/.../contents/...`. Le seam de test `routers/private.TestSeamPostReceiveLFSPromotion` saute la promotion post-receive. Avant le changement de service, le serveur renvoyait 201 et le commit candidat était bien la tête du ref, mais `GetLFSMetaObjectByOid` retournait `LFS Meta object does not exist`. Le run rouge échoue à cette assertion avec code 1. Le journal est `red.log`; `red.exit` enregistre le code.

## Changement expérimental, échec récupérable et résultat vert

Dans `services/repository/files/update.go::ChangeRepoFiles`, les pointeurs LFS de la requête sont collectés pendant l'écriture temporaire; après création du commit, leurs associations sont enregistrées comme pending avant `TemporaryUploadRepository.Push`. Après le succès du push, le service appelle `PromoteLFSPendingMetaObjects` avec la même clé d'opération, avant de construire et renvoyer la réponse de fichier. La promotion est idempotente avec celle du hook normal. Si elle échoue après l'acceptation du commit, le service renvoie une erreur qui nomme le commit et l'opération pending à reprendre, sans prétendre annuler le push.

Le test `TestPOCGenericFileLFSAPI201IsReturnedAfterPromotion`, avec la promotion du hook toujours sautée, observe alors : ref égal au commit annoncé, association active présente, zéro ligne pending, et GET LFS HTTP 200 avec les octets attendus.

Le second test `TestPOCGenericFileLFSAPIErrorLeavesRecoverablePendingOperation` ajoute un trigger SQLite temporaire qui rejette l'INSERT de l'association active. Le push a déjà installé le candidat; le service répond HTTP 500, conserve l'association pending, et son journal nomme le commit ainsi que la clé d'opération à reprendre. Le GET LFS retourne 404 avant reprise. Après retrait du trigger, l'appel direct au récupérateur existant confirme le candidat dans le ref, promeut une opération, enlève la ligne pending et le GET retourne 200 avec les octets exacts. Le trigger est aussi supprimé en nettoyage.

Les deux tests passent ensemble 2/2, et deux exécutions ciblées antérieures du test positif passent chacune 1/1. Les sorties sont `green.log`, `green-rerun.log`, `recovery-green.log` et `combined-final.log`, avec codes enregistrés; les commandes sont dans `commands.txt`.

Le harnais imprime encore `FATAL ERROR` pour des erreurs `context canceled` émises par des tâches de nettoyage après les assertions. Le test Go affiche explicitement `PASS`, le paquet finit `ok` et le processus retourne 0. Cette sortie de journalisation est conservée intégralement et ne doit pas être décrite comme un run sans bruit.

## Portée et limites

- Il s'agit du writer générique `ChangeRepoFiles` exercé via la véritable route HTTP dans un serveur d'intégration, avec le seam déterministe qui saute la promotion du hook et une panne d'INSERT SQLite injectée. Ce n'est ni une panne disque/processus ou un arrêt du serveur, ni une vérification de tous les writers Forgejo.
- Le récupérateur existant est appelé directement par le test après suppression du trigger; aucun dispatcher ou worker automatique n'est démontré. Les lignes pending et les octets peuvent rester après un push refusé ou une erreur durable de promotion.
- Le test ne démontre pas un fence commun couvrant branches, renommages/suppressions, push interne, merge/rebase et API. Il ne valide pas PostgreSQL/MySQL, un cluster, un stockage distant, une reprise après SIGKILL, ni les coûts upstream d'un patch maintenu.
- Le scratch contient des modifications d'autres POC et l'identité de commit indique seulement le commit de base. Les empreintes finales des deux fichiers touchés sont dans `source-sha256.txt`.
- Aucune intégration AgentOS, RAG, mémoire ou LLM, aucun parcours utilisateur complet additionnel et aucun benchmark comparatif ne sont établis par ce POC.

Ce résultat réduit une fenêtre d'acquittement dans un seul writer; il ne justifie pas encore une modification durable du noyau Forgejo. Une adoption requiert le ledger/promotion récupérable, l'audit et le fence partagé de tous les writers, des tests de panne/reprise et une estimation de maintenance upstream.
