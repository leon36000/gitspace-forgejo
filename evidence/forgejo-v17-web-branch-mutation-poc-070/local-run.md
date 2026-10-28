# FORGEJO-V17-WEB-BRANCH-MUTATION-POC-070 — résultat local

Date : 25 octobre 2026. Le scratch `/tmp/forgejo-v17-base-cas-poc` part du tag officiel Forgejo `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, et contient les changements de diagnostic des POC antérieurs. Le lot ajoute seulement un test d’intégration au scratch; aucune source Forgejo n’est copiée dans le dépôt produit.

## Question examinée

La politique expérimentale `RequireExpectedBaseCommitID`, appliquée au hook pre-receive, bloque-t-elle aussi les modifications de branches via les routes Web de renommage et de suppression ?

## Comportement observé

`TestPOCExpectedBasePolicyDoesNotGateWebBranchMutationRoutes` démarre l’application d’intégration Forgejo avec SQLite et `LocalStorage`. Il crée deux branches, conserve leur SHA observé, puis fait avancer chacune par l’éditeur Web avant d’activer la politique strictement dans l’unité du dépôt.

- La route `POST /user2/repo1/settings/rename_branch` renvoie `303`. Le nom de branche change et le nouveau ref conserve le commit courant `c2762e85f5be644bdacb1e59c88704cbce3f9c5f`, alors que le SHA observé avant l’avance était `65f1bf27bc3bf70f64657658635e66094edbcb4d`.
- La route `POST /user2/repo1/branches/delete` renvoie `200` et supprime le ref après qu’il est passé de `65f1bf27bc3bf70f64657658635e66094edbcb4d` à `b6d585aa5751f80ae446b62bd555672ab93a16a7`.

Le test ciblé réussit : `1 passed`, paquet `tests/integration` en `1.880s`, Go `1.27.3 linux/arm64`, tag `sqlite`, `GOPROXY=off`. Les handlers Web appellent directement `repository.RenameBranch` et `repo_service.DeleteBranch`; ils ne transmettent pas de base attendue au hook. Le hash avant avance est donc une observation du test, pas un champ réellement envoyé par le formulaire.

## Commande et fidélité de la preuve

Commande : voir [`commands.txt`](commands.txt). Le terminal a tronqué l’essai initial avant archivage du journal complet. [`local-run.log`](local-run.log) conserve les lignes de résultat et les SHA observés que le terminal a rendus sans troncature; le test source reste disponible dans le scratch et peut être reproduit avec la commande enregistrée. L’empreinte du test et celles des symboles examinés sont dans [`source-sha256.txt`](source-sha256.txt).

## Portée et décision

Ce test établit deux autres routes directes qui ne traversent pas `assertExpectedBaseCommitID`; il ne mesure pas une course entre le chargement d’une page Web et son POST, ni l’ensemble des writers Forgejo. Combiné aux essais API antérieurs, il interdit de considérer le hook receive comme une précondition universelle. En particulier, un protocole LFS ne peut pas marquer une opération `rejected` ou supprimer son association pending sur la seule base d’un hook que certains writers contournent.

Décision : conserver Forgejo comme dépendance isolée, ne pas adopter le patch CAS et ne pas annoncer que la politique expérimentale couvre tous les writers. La prochaine vérification doit compléter la matrice des routes directes et tester un fence commun aux changements de refs avant de rendre terminale une opération LFS. Les fichiers modifiés du scratch ne donnent pas encore un coût upstream isolé; aucun chiffre de maintenance n’est avancé.

Limites : le scratch est fortement modifié par les POC LFS/CAS, la politique est activée directement dans le test (son administration et sa migration ne sont pas examinées), seul SQLite est utilisé, et l’essai ne teste ni plusieurs processus/instances ni stockage distant. Il ne s’agit ni d’un déploiement ni d’une reproduction sur Forgejo amont intact.
