# Rapport local — matrice d’effets des writers Forgejo (FORGEJO-V17-WRITER-EFFECT-MATRIX-001)

Date : 25 octobre 2026. Objet : classement statique des candidats de ref writer et décision d’intégration. Il ne s’agit pas d’un nouveau test d’intégration.

## Source et méthode

- Forgejo officiel `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Les call sites cités sont lus dans les objets `HEAD` avec `git show`/`git grep`, pas déduits des modifications du patch scratch.
- Scratch : `/tmp/forgejo-v17-base-cas-poc`, base identique, worktree POC fortement modifié. L’état et les nombres de modifications sont dans `scratch-state.txt`; les SHA-256 du contenu présent y sont dans `source-hashes.txt`, tandis que `upstream-source-blobs.txt` référence les blob IDs upstream exacts.
- Scan lexical : commande complète dans `commands.txt`; 35 chemins, mêmes chemins que le scan POC-075. Liste brute `writer-scan-paths.txt`, comparaison `previous-scan-compare.txt`, call sites candidats `write-call-sites.txt`, classification vérifiée exactement contre les 35 chemins dans `writer-classification.tsv`, analyse `writer-effect-matrix.md`.
- Outils d’examen : Git, `rg`; aucun test ni build exécuté pour la matrice statique. La consolidation du 25 octobre relit en plus les enregistrements API/Web et les handlers de branche par `git show HEAD:<path>`; commandes exactes, blob IDs et SHA-256 sont conservés dans `commands.txt`, `upstream-source-blobs.txt` et `source-hashes.txt`. Elle intègre WRITER-RESTORE-POLICY-071 comme preuve antérieure distincte, sans la relancer. Les preuves d’exécution demeurent des tests du scratch modifié, et non de Forgejo amont intact.

## Résultat

Le scan mélange des writers de refs utilisateur, des mutations directes sans receive-pack, des refs PR/temporaire, des destinations distinctes (fork, miroir, wiki et index Cargo), des wrappers génériques et des faux positifs. Une règle portée seulement par `pre-receive` ne couvre pas ce périmètre. Les endpoints API de branche utilisent tous le contrôle général d’écriture de code; l’existence d’un rôle capable de merger mais incapable de ces autres mutations n’a pas été démontrée. Le protocole LFS de staging du scratch ne couvre pas encore les writers audités comme une autorité unique.

Décision : préserver Forgejo en dépendance avec la couche agentique autour, ne pas fusionner les sources et ne pas adopter le fork/pach CAS actuel. La couche externe n’est pas encore autorisée à prétendre que le merge est atomiquement contrôlé. Prochaine preuve : exercice d’ACL non-admin de bout en bout; puis décider entre autorité unique réellement isolée et interface serveur centrale. La comparaison des modifications du scratch n’est pas une estimation du coût upstream.

## Vérifications et limites

- Compte du scan initial : 35 chemins; cela ne signifie pas 35 écrivains et le scan n’est pas exhaustif. La consolidation ciblée vérifie les enregistrements de routes API/Web de branche, mais les handlers tag, plugins/hooks, chemins dynamiques et le call graph complet ne sont pas examinés.
- État laboratoire au relevé : 58 entrées Git de statut et 19 fichiers Go suivis modifiés, toutes générations de POC confondues.
- Aucun binaire amont compilé ou lancé; aucun serveur Forgejo vivant, PostgreSQL, cluster, LLM ou service externe sollicité. Aucune donnée privée et aucun secret. Aucun chiffrage humain/upstream avancé.
- Forgejo n’a pas été modifié par ce lot; aucun fichier source Forgejo n’est copié dans le produit.
