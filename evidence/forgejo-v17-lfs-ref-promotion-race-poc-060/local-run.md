# POC-060 — déplacement de la ref entre observation et promotion LFS

Date : 25 octobre 2026. Scratch : `/tmp/forgejo-v17-base-cas-poc`, Forgejo officiel v17.0.0, commit de base `0d791421f85ca13a63b665053d81ea765fbbc571`. Ce POC teste uniquement le prototype de récupération ajouté au scratch; aucun code Forgejo n’a été copié au produit.

## Hypothèse et observation

Dans `services/pull/lfs_recovery.go::recoverLFSPendingMetaObjectsPage`, `for-each-ref --contains=<candidate>` établit d’abord la reachability de la ref cible, puis `git_model.PromoteLFSPendingMetaObjectByID` démarre sa transaction DB sans relire la ref. Le helper modèle (`models/git/lfs.go`) publie l’association active et supprime l’association pending dans la DB; cette transaction n’est pas atomique avec les refs Git.

Le test `TestPOCRecoveryPromotesFromARefSnapshotThatIsStaleBeforeDBPromotion` met une barrière contrôlée autour du vrai `git for-each-ref`. Il attend que le résultat confirme `refs/heads/main`, déplace alors cette ref vers un commit racine indépendant, vérifie avec le vrai Git que la ref ne contient plus le candidat, puis libère le récupérateur. Résultat : `RecoverLFSPendingMetaObjectsCheckpointPage` retourne sans erreur avec une association promue, le pointeur LFS devient actif et aucune ligne pending ne reste, alors que `refs/heads/main` vise le commit indépendant. Le log final imprime les deux SHA et `promoted=1`.

Le test passe parce qu’il reproduit le résultat observé; ce `PASS` n’est pas une approbation de ce comportement. Il démontre une fenêtre de cohérence entre le snapshot Git et la promotion DB, pas une fuite inter-dépôts ou un accès non autorisé. Le candidat avait été présent sur la ref au moment de l’observation; la politique souhaitée pour un déplacement concurrent ou une publication fugitive reste une décision de conception.

Commande finale :

```text
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags sqlite,forgejo_poc_lfs_recovery_multiprocess ./services/pull -run '^TestPOCRecoveryPromotesFromARefSnapshotThatIsStaleBeforeDBPromotion$' -count=1 -v -timeout=10m
```

Résultat : scénario ciblé `PASS` 1/1, paquet `services/pull` réussi en 1,604 s. Le wrapper ne masque pas le binaire Git pour les autres commandes; la barrière capture la sortie avant le déplacement. Les versions et SHA-256 des sources sont dans `tool-versions.txt` et `source-hashes.txt`; log final dans `ref-race-final.log`.

## Portée et suite

- Un seul run sur SQLite fichier, un hôte, dépôt Git nu réel. Pas de serveur Forgejo, PostgreSQL/MySQL, véritable hook de push, multi-processus concurrent sur la DB, arrêt forcé ou analyse exhaustive des writers.
- Le test ne corrige pas la course. Une seconde lecture du ref réduirait la fenêtre sans rendre la comparaison Git+DB atomique. Un contrat robuste doit définir si une association reste active quand le candidat a été brièvement publié puis retiré, et comment les readers/GC traitent les objets correspondants.
- Étudier le rôle de la promotion post-receive, une revalidation sérialisée du ref et les options de verrouillage/fencing, avec tests d’interruption avant/après l’effet. Toute solution gardera une stratégie de reprise idempotente et un nettoyage/rétention bornés.
- POC-058/059 restent des tests de harness; le dispatcher migration complet et le serveur ne sont pas démarrés. Aucun worker n’est proposé/adopté et aucun code n’est porté au produit.
- Pas de LLM, service externe, secret, dépense, publication ou changement de production.

Prochaine action : choisir et tester la sémantique de promotion lorsque le ref a bougé, sans présumer qu’une double lecture suffit. Continuer les refus composites produits non démontrés; le parcours LLM et la comparaison contrôlée restent ouverts.
