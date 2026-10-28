# CONTINUATION-318 — dismissal concurrente avant un merge Web

Date : 27 octobre 2026.

## Question

Le chemin Web de fusion applique-t-il une révocation d’approbation faite après le contrôle `CheckPullMergeable` mais avant que le même formulaire Web ait terminé le merge?

## Source et environnement

- Forgejo v17.0.0, commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`.
- Scratch détaché `/tmp/forgejo-v17-merge-review-race-001`; il contient des hooks de test API et Web ajoutés aux lots CONT-316/317/318. Le code produit et le serveur amont intact n’ont pas été modifiés.
- Test `tests/integration/web_pull_merge_race_test.go::TestWebMergePullRequestCanUseApprovalBeforeConcurrentDismissal` utilise SQLite (`tests/sqlite.ini`), la route Web avec une session authentifiée et la route API de dismissal; Go `1.27.3 linux/arm64`, Git `2.43.0`, `CGO_ENABLED=1`.

## Source examinée

Dans le HEAD officiel, `routers/web/web.go:1755` enregistre le formulaire POST `/merge`. `routers/web/repo/pull.go::MergePullRequest` appelle `CheckPullMergeable` à la ligne 1398 avant la branche manuelle puis appelle `pull_service.Merge` à la ligne 1487 sur le chemin normal. Le hook test-only a été inséré après le succès de ce check, dans la copie scratch. Les empreintes HEAD et scratch sont dans `source-sha256.txt`.

## Protocole et résultat

Le test crée une protection du dépôt à une approbation, autorise la revue de `user2`, active `ApplyToAdmins` et `DismissStaleApprovals`, crée une PR et une revue officielle exacte. `user1` poste par formulaire Web les paramètres `do=merge` et le SHA `head_commit_id`. Une barrière retient la requête après le contrôle. L’owner appelle alors la route API `POST .../reviews/{id}/dismissals`; elle retourne 200 avec `dismissed=true`. Le formulaire Web est relâché, retourne HTTP 200, et les GET API confirment `HasMerged=true` et la même revue `dismissed=true`.

Le test passe cinq fois avec `-count=5`; il passe une fois avec `-race -count=1`. Les sorties complètes se trouvent dans `integration-test-5x.log` et `integration-test-race.log`.

Conclusion précise : sur ce chemin Web et dans la copie instrumentée, le préflight ne protège pas contre le dismissal pendant la fenêtre. La révocation est faite par l’autre canal API; la route Web a accepté le merge précontrôlé ensuite.

## Tentative corrigée et limites

Le premier `go test` a échoué à la compilation à cause d’un import `apirepo` inutilisé; cet import a été retiré, `gofmt` exécuté et le test vert a ensuite été répété cinq fois et sous `-race`. Le premier message est conservé dans `attempts.txt`.

Le test est un run du serveur d’intégration avec SQLite et push local, mais un hook test-only contrôle le moment de la concurrence. Ce n’est pas un test d’un build amont intact/production. Les routes auto-merge, `manually-merged`, force, Actions et les autres writers ne sont pas testées; aucune assertion sur PostgreSQL, cluster ou déploiement. Aucun code Forgejo n’est copié/adopté dans le produit, aucun LLM/service externe/secret/coût n’est utilisé. La licence de Forgejo n’est pas impliquée dans le produit, qui ne réutilise pas ce code.

Suite : lire les call sites d’AutoMerge, manuel, force et Actions; tester seulement les voies qui ont une frontière indépendante. Mesurer ensuite le changement serveur commun requis avant de considérer un fork limité; la preuve ne justifie pas à elle seule l’adoption du POC CAS.
