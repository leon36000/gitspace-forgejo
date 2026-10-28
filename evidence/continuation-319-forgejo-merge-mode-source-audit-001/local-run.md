# CONTINUATION-319 — audit ciblé des modes de merge Forgejo

Date : 27 octobre 2026.

## Portée

Lecture statique seulement du code officiel Forgejo v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch `/tmp/forgejo-v17-merge-review-race-001` est détaché à ce commit mais contient les hooks et tests des CONT-316 à 318; les fichiers lus pour cette analyse ont été extraits avec `git show HEAD:<path>`, sans lire les versions modifiées. Aucun test Go, binaire Forgejo, runner Actions ou instance n’a été exécuté dans ce lot.

## Résultats de lecture

1. **Mode manuel.** `CheckPullMergeable` valide d’abord l’état de PR et `IsUserAllowedToMerge`, puis retourne immédiatement pour `MergeCheckTypeManually`; les protections de branche, approbations, statuts, signature, WIP et dépendances ne sont pas vérifiés dans cette branche. `MergedManually` vérifie le style autorisé et que le commit envoyé se résout puis est accessible sur la branche de base; il marque la PR fusionnée avec ce commit. Ce code ne lie pas le commit à l’arbre candidat de la PR. Le comportement problématique d’un commit de base concurrent et d’un fichier PR absent a déjà été testé dans [POC-002](../forgejo-v17-base-cas-poc-002-lfs-residue/local-run.md); CONT-319 ne le rejoue pas.

2. **AutoMerge.** En mode de planification, `CheckPullMergeable` permet de continuer malgré `ErrDisallowedToMerge` pour enregistrer le merge différé. Le worker relit le commit de tête et exige le SHA transmis par la file, vérifie les statuts, recharge le compte et ses permissions, fait un `CheckPullMergeable` général, supprime la planification puis appelle `pull_service.Merge`. Cette revalidation réduit le risque de déclencher le merge sur un event obsolète, mais le dernier check et le changement de ref ne sont pas démontrés atomiques. Pas de race spécifique au worker testée ici.

3. **Force merge.** Les routes API et Web transmettent `form.ForceMerge` à l’argument `adminSkipProtectionCheck`. Le checker n’efface `ErrDisallowedToMerge` par ce paramètre que si `doer.IsAdmin`, ou si l’auteur est admin du dépôt et `ApplyToAdmins` est faux. Un booléen client seul ne confère donc pas ce droit; un credential d’admin exposé à l’agent reste une frontière critique.

## Limites et prochaine vérification

Lecture de symboles précis, pas une revue complète des routeurs, des actions, hooks de push, protecteurs branch pre-receive ou permissions de tous les writers. Les tests CONT-316 à 318 couvrent les routes API/Web normales avec hooks de test, pas AutoMerge. Suite utile : placer une barrière dans le worker AutoMerge entre `CheckPullMergeable` et `Merge`, révoquer l’avis ou modifier la policy, puis relire PR/ref; vérifier également si les runners Actions ou les utilisateurs privilégiés peuvent modifier les refs par une voie distincte. Aucune modification ou copie Forgejo n’est adoptée.
