# Forgejo v17 — course du merge PR entre lecture du ref et publication — POC-075

Date : 25 octobre 2026. Le scratch /tmp/forgejo-v17-base-cas-poc est basé sur le commit amont v17.0.0 0d791421f85ca13a63b665053d81ea765fbbc571, mais contient de nombreux changements POC non adoptés. Aucun code Forgejo n’a été copié dans le dépôt produit.

## Question

Le chemin de fusion serveur d’une PR protège-t-il l’état merged et la publication des associations LFS contre un déplacement de la branche cible après l’observation du hook ?

## Parcours exécuté

tests/integration/lfs_api_ack_order_poc_test.go::TestPOCPullMergeCanMarkMergedAndPublishLFSAfterTargetRefMoves crée un dépôt de base avec .gitattributes, un fork avec un vrai contenu LFS, une PR, puis déclenche la route Web réelle de fusion. Les services de fusion, le push Git et les hooks pre/post-receive sont exercés. Une seam de test déplace refs/heads/master vers son commit de base après que le hook a lu le commit candidat et avant les écritures SQL ultérieures.

Résultat reproductible : la route répond HTTP 200. Au readback, refs/heads/master pointe toujours vers le commit de base; la PR est marquée fusionnée et son MergedCommitID diffère du ref courant; l’association LFS du contenu est active, mais le commit courant ne contient pas le fichier LFS. Le test ciblé passe 1/1 parce qu’il affirme cette incohérence observée; ce n’est pas une réussite de sécurité. La sortie exacte est dans final-run.log; exit code 0 dans exit-code.txt.

## Interprétation et décision

Dans l’arbre scratch, le service prépare et pousse la fusion à services/pull/merge.go:360-404, puis promeut l’opération LFS à :437-440. Le hook lit le ref avant la promotion à routers/private/hook_post_receive.go:187-199; handlePullRequestMerging stocke le SHA des données du push comme commit de fusion à :410-445; la boucle PR promeut encore les métadonnées d’après l’événement à :247-259. La seam déterministe montre que cette chaîne n’a pas de fence commun couvrant ref Git, état PR SQL et association LFS SQL.

Une précondition passée au pre-receive, ou une relecture seule avant la transaction DB, ne suffit pas à établir que le commit est toujours la cible au moment où l’état de fusion et LFS sont publiés. Un protocole qui prétend couvrir les merges doit relier l’identité de la PR, le candidat, la base et l’autorité serveur dans une mutation/fence commune, puis vérifier le résultat réel de fusion. Il faut aussi empêcher qu’un writer concurrent change le ref hors de ce protocole.

## Inventaire complémentaire et compatibilité

Le scan élargi ajoute les commandes git.NewCommand(..., "push") au repérage initial. Il donne 35 chemins lexicaux, mais plusieurs faux positifs sont des constantes de webhook, des schémas ou des wrappers; ce total n’est pas le nombre de writers effectifs. Il révèle notamment le chemin cible services/pull/merge.go, absent du premier scan, et le chemin de tête PR services/pull/update_rebase.go. La liste brute et la classification prudente sont dans writer-scan-paths.txt et writer-scan.txt.

La compatibilité et la maintenance resteraient multi-sous-systèmes : refuser ou accepter une mutation exige des règles cohérentes dans les routes API/Web, pushes Git, merge/rebase/AutoMerge, opérations directes et imports/mirrors. Une autorité unique protégée par permissions serait moins intrusive si elle pouvait effectivement interdire les contournements; sinon il faut concevoir et maintenir un protocole serveur partagé. Aucun chiffrage en heures fiable n’est possible avec ce POC.

## Vérification et limites

Commande, versions et empreintes se trouvent dans commands.txt, toolchain.txt et source-hashes.txt. Go 1.27.3 linux/arm64, Git 2.43.0, SQLite, GOTOOLCHAIN=local, GOPROXY=off. gofmt -d sur le fichier de test est vide. L’essai est un seul test dans un processus d’intégration avec déplacement de ref synchronisé à une seam; pas de deuxième processus Forgejo, PostgreSQL, cluster, mesure de performance ou déploiement. L’arbre est modifié au-delà de ce test. Cette preuve caractérise le patch scratch; elle n’attribue aucun défaut au commit upstream propre. Le POC ne justifie pas l’adoption d’un fork.
