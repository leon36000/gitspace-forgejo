# FORGEJO-V17-LFS-STALE-DISCARD-POC-068 — résultat local

Date : 25 octobre 2026. Dépôt scratch `/tmp/forgejo-v17-base-cas-poc`, HEAD de base Forgejo officiel v17.0.0 : `0d791421f85ca13a63b665053d81ea765fbbc571`. Le checkout scratch est fortement modifié; le test ajouté est sous build tag et aucun code n’est porté au produit. Les empreintes des sources concernées figurent dans `source-sha256.txt`.

## Question vérifiée

Un nettoyeur peut-il supprimer sans risque un stage après avoir vu son candidat absent de la ref cible, si un writer publie ce candidat avant la promotion post-receive ?

## Résultat observé

`TestPOCStaleMissingCandidateDiscardCanBlockLaterPromotion` utilise SQLite de test, un dépôt Git bare local et les fonctions réelles du scratch `StageLFSMetaObject`, `RecoverLFSPendingMetaObjectsPage`, `DiscardLFSPendingMetaObjects` et `PromoteLFSPendingMetaObjects`. Le candidat est absent de `refs/heads/main`; la récupération le laisse en attente (`DeferredAssociations=1`). Le test met ensuite la ref cible à ce candidat, appelle `Discard` avec la même clé d’opération avant la promotion post-receive, puis rejoue la promotion. Le candidat reste sur `main`, mais l’association `LFSMetaObject` est absente, le compteur de stages est nul et la ligne d’identité d’opération reste orpheline.

Le test ciblé passe avec `-tags sqlite,forgejo_poc_lfs_stale_cleanup` : 1 réussi, 0 échec, 0 ignoré, paquet `services/pull` en 0,573 s; `gofmt -d` ne produit aucune différence. Un premier essai de compilation échouait à cause d’une importation inutilisée et de la réutilisation d’une variable `RunStdError` dans le test; ces erreurs d’auteur de test sont conservées dans `initial-test-compile-failure.log`, corrigées avant le run vert.

## Interprétation et limites

Il s’agit d’un interleaving diagnostique, pas d’un défaut observé dans un worker actif : le scan négatif vient du chemin de récupération réel, mais le nettoyeur n’existe pas; le test orchestre lui-même l’appel à `Discard` entre publication de ref et promotion. Il n’exécute ni hook HTTP Forgejo, ni writer API réel, ni stockage objet. Il démontre seulement que les fonctions DB actuelles ne rendent pas une telle suppression sûre par elles-mêmes.

Dans le scratch, `LFSPendingMetaOperation` est une identité immuable sans état terminal; `DiscardLFSPendingMetaObjects` supprime les associations pending sans revalider la ref ni acquérir un fencing partagé avec les writers. Le recovery actuel ne supprime pas : sans témoin de reachability, il diffère la ligne. `rg` ne trouve aucun appelant de `RecoverLFSPendingMetaObjects*` dans le code de production; les occurrences sont les helpers/tests du scratch. Un candidat absent ne doit donc pas être classé automatiquement comme rejeté. Avant tout worker, il faut définir des états d’opération terminalisables et un verrou/fence partagé avec chaque writer de ref; un CAS SQL seul ne sérialise pas un `git update-ref`. La suppression physique du blob OID partagé reste une frontière distincte, car `ObjectStorage.Delete(path)` ne porte pas de génération.

Le test est en build tag diagnostic et n’ajoute aucun comportement runtime Forgejo. Il ne démontre ni un serveur installé, ni une intégration de production, ni un parcours agent/LLM, ni un gain comparatif. Aucun modèle, service externe, secret ou coût réseau n’est utilisé (`GOPROXY=off`).

## Fichiers de preuve

- `commands.txt` : commandes exactes et périmètre.
- `local-run.log` : run ciblé final.
- `initial-test-compile-failure.log` : erreur d’auteur de test avant correction.
- `test-result.txt` et `tool-versions.txt` : résultat, environnement et limites.
- `source-sha256.txt` : HEAD et empreintes des sources du scratch.
- `SHA256SUMS` : intégrité des fichiers du rapport.
