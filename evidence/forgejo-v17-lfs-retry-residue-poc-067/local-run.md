# POC-067 — retry d’un writer LFS après refus pre-receive

Date : 25 octobre 2026. Scratch Forgejo officiel v17.0.0, HEAD de base `0d791421f85ca13a63b665053d81ea765fbbc571`; les sources testées comprennent les modifications locales des POC antérieurs. Aucun code du scratch n’est copié dans le produit.

## Question

Après qu’une écriture API de fichier LFS est refusée au pre-receive, une demande identique peut-elle réussir plus tard, et que devient le staging pending attaché au premier commit candidat ?

## Scénario et résultat

Le nouveau test d’intégration `TestPOCGenericFileLFSRetryAfterPolicyRefusalLeavesFirstStagePending` traverse l’API HTTP, l’écriture de fichier temporaire, le hook pre-receive, le push local, le hook post-receive, la base SQLite et le lecteur LFS.

1. Il active temporairement `RequireExpectedBaseCommitID`, puis envoie une création de fichier `.bin` LFS. L’API retourne 403; la branche ne bouge pas, aucune association active n’existe et une association pending pour le candidat refusé est conservée.
2. Il enlève cette politique de fixture, attend deux secondes pour obtenir un SHA de commit différent, puis renvoie la même demande API avec le même chemin, contenu et message. L’API retourne 201, le ref atteint le nouveau commit et l’objet LFS devient lisible avec les octets attendus.
3. L’ancien SHA candidat n’existe pas dans le dépôt de base (`git cat-file -e` échoue). L’ancien enregistrement pending demeure. La récupération n’en déduit pas une publication : elle retourne une erreur Git `no such commit`, compte l’association comme différée et la laisse pending. La ligne n’empêche pas la nouvelle association active ni le GET LFS du nouvel écrit.

Le test ciblé final passe 1/1 sous SQLite et Go 1.27.3/linux/arm64. Sa première exécution échouait seulement parce que le test supposait à tort que la récupération devait renvoyer `nil` pour le candidat absent. Le contrat scratch existant `TestRecoverLFSPendingMetaObjectsDefersMissingCandidate` attend explicitement une erreur fail-closed dans ce cas; le test de caractérisation a été corrigé pour vérifier cette sémantique, sans changer de code Forgejo.

## Résultat et décision

Cette observation précise le coût du refus puis reprise : le deuxième commit est servi correctement, mais le premier stage forme un résidu durable que le récupérateur ne peut pas résoudre sans politique de rétention/nettoyage. L’erreur est un refus sûr, pas un succès falsifié; son traitement par un futur worker doit éviter une boucle bruyante et ne peut supprimer le stage sans prouver qu’aucun writer concurrent ne publiera le même candidat. Le test n’implémente ni worker, ni ledger durable, ni suppression.

La fixture active directement une option de dépôt dans le test; elle ne démontre pas son interface opérateur, son déploiement, la totalité des writers ou une course réelle entre utilisateurs. Un délai de deux secondes crée volontairement un nouvel identifiant de commit; le test ne prétend pas valider la déduplication d’une requête simultanée. Go/Git/SQLite sont locaux, sans réseau externe, modèle LLM, coût, production ou publication. Le patch CAS et tous les autres POC restent non adoptés; aucun coût de rebase upstream n’est chiffré.

## Reproduction

La commande finale est dans `commands.txt`; le journal vert et son code de sortie sont `integrated-e2e.log` / `integrated-e2e.exit`. La première exécution avec l’hypothèse trop stricte est conservée dans `initial-assumption-failure.log` / `initial-assumption-failure.exit`. Les contrôles finaux sont `gofmt-diff.log` et `diff-check.log`. L’empreinte SHA-256 de chaque fichier source pertinent est dans `source-sha256.txt`; la vérification est dans `source-hash-check.txt`.

Le journal de test contient le diagnostic attendu du pre-receive 403 et une ligne `FATAL ERROR` du testlogger pour cet appel refusé, mais l’assertion, le retry 201, la récupération fail-closed, le GET LFS 200 et le processus terminent PASS avec code 0. Cette sortie de harness est conservée, pas reclassée comme absence d’erreur.
