# POC-063 — création de branche web sous précondition stricte

Date : 25 octobre 2026. Forgejo scratch : /tmp/forgejo-v17-base-cas-poc, base officielle v17.0.0, commit 0d791421f85ca13a63b665053d81ea765fbbc571. Le scratch est fortement modifié par des expériences précédentes; il ne représente pas un checkout pristine. Aucun changement de production Forgejo n’est fait dans ce lot, et aucun code Forgejo n’est copié dans le produit.

## Résultat

Le test TestPOCExpectedBasePolicyKeepsWebBranchCreateActionable active RequireExpectedBaseCommitID directement dans la configuration de la fixture SQLite, puis soumet le formulaire web existant pour créer une branche depuis la branche par défaut. Le hook serveur de réception expérimental refuse le push avec 409 car le flux ne transmet ni précondition attendue ni contexte PR. La route web répond toutefois 303, redirige vers le dépôt, puis affiche dans le message la consigne « refresh and retry through the server merge path ». Le test vérifie également que la ref Git et la ligne de branche SQLite restent absentes.

Commande : GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 -v -timeout=20m -tags sqlite ./tests/integration -run '^TestPOCExpectedBasePolicyKeepsWebBranchCreateActionable$'. Résultat 1/1, code 0; le log observe le 409 interne, le POST web 303 et le GET de redirection 200. Les sources, test et dépendances sont identifiés dans source-sha256.txt; les versions figurent dans tool-versions.txt.

Le résultat complète la comparaison API/UI : POC-062 a observé que la création API appelle le même service mais transforme le refus du hook en HTTP 500 et laisse la branche absente; cette tranche montre que la web UI traduit le même refus en message visible et actionnable. Elle ne montre pas que la création peut réussir sous cette politique ni qu’un client puisse fournir une base attendue.

## Portée et limites

La modification de ce lot est un test d’intégration scratch seulement. Les fichiers de route API et web et le service partagé sont inchangés dans le scratch; le hook de pré-réception, lui, contient toujours le patch expérimental des POC précédents. La politique n’est pas activée par un réglage d’interface testé et les routes autres que celle-ci ne sont pas couvertes. Le test utilise SQLite et le serveur d’intégration local Forgejo; il n’exerce ni instance de production, ni agent, LLM, Actions runner, credential réel ou service externe. Le résultat caractérise une incompatibilité de création sans contrat de précondition, pas un comportement Forgejo upstream sans patch.

Ce cas ajoute une distinction de compatibilité à la matrice : rejet sûr de la création, mais erreurs différentes selon le client (API opaque HTTP 500; web UI avec message réparable). Il ne justifie pas à lui seul un fork; un contrôle cohérent reste dépendant d’un contrat de base attendu, de l’ensemble des writers et du coût de maintenance upstream.
