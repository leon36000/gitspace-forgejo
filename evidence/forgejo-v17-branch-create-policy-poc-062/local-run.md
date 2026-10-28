# POC-062 — création de branche refusée par la précondition stricte

Date : 25 octobre 2026. Scratch : `/tmp/forgejo-v17-base-cas-poc`, basé sur Forgejo officiel v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le worktree Forgejo est fortement modifié par les POC antérieurs; ce lot ajoute un test d'intégration seulement et ne modifie aucun fichier de production. Aucun code Forgejo n'est copié dans le produit.

## Observation

Le test active directement `RequireExpectedBaseCommitID` sur l'unité Pull Requests de `user2/repo1`, puis envoie la création API d'une branche depuis le commit courant de la branche par défaut. `CreateBranchRepoOption` expose le nom de branche et la source (`old_branch_name` ou `old_ref_name`), mais pas de SHA de source attendu. Le service `CreateNewBranchFromCommit` pousse localement avec `PushingEnvironment`, sans contexte serveur de PR ni `ExpectedBaseCommitID`.

Le hook `preReceiveBranch` refuse donc l'ajout du nouveau ref avec HTTP 409 en expliquant qu'aucune précondition/context PR n'est présent. La route `routers/api/v1/repo/branch.go::CreateBranch` convertit ce `PushRejected` non classé en HTTP 500. Le ref cible reste absent. C'est une incompatibilité de la politique expérimentale et un défaut d'actionnabilité de son mapping d'erreur, pas le comportement du Forgejo amont sans ce patch. L'état de la politique est configuré directement par le test; aucune interface utilisateur/API ou migration n'est exercée.

## Vérification

Le premier test attendait la création habituelle HTTP 201 et échoue comme prévu : la réponse réelle est HTTP 500 après refus du hook. Le test est ensuite conservé comme test de caractérisation : il attend explicitement HTTP 500 et vérifie que le ref n'a pas été créé. Run final : 1/1, code 0. Logs bruts : `red.log` et `final.log`.

```sh
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -count=1 -v -timeout=20m -tags sqlite ./tests/integration \
  -run '^TestPOCExpectedBasePolicyMakesAPIBranchCreateFailClosedButOpaque$'
```

`gofmt -d` et `git diff --check` sur le seul test sont sans sortie, code 0. Le test et les sources touchées sont identifiés dans `source-sha256.txt`; les outils sont dans `tool-versions.txt`.

Une première commande de lecture des empreintes source a été lancée depuis le sous-dossier de preuves, alors que les chemins de `source-sha256.txt` sont relatifs à la racine du scratch; elle a seulement échoué à ouvrir ces chemins. Relancée depuis la racine du scratch, les neuf empreintes source/test/log passent. Le manifeste local du lot passe 5/5.

## Coût et portée

Le champ de base attendu n'est absent que d'un seul contrat JSON d'API dans ce cas observé, mais une intégration cohérente demanderait au minimum de définir sa sémantique pour la ref source d'une création, de le transporter à travers le service de push, de distinguer ce cas d'un update de ref dans le hook et de renvoyer une erreur HTTP actionnable. La web UI partage le helper, sans test comportemental dans ce lot. Ce décompte est une borne de surfaces observées, pas une estimation d'heures; le coût de rebase/maintenance upstream reste non chiffré.

Le POC n'a pas audité tous les writers, tags, wikis, miroirs, imports, AutoMerge ou workers. Les refus API branch-delete et la réussite du renommage hors hook sont déjà établis par POC-061; ils ne sont pas rejoués ici. Aucun LLM, credential réel, service externe, dépense, production ou publication n'est utilisé.
