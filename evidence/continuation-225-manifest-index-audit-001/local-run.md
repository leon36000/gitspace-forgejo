# CONTINUATION-225 — contrôle des index de preuves append-only

Date : 27 octobre 2026. Contrôle effectué dans le worktree local très modifié à HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun fichier d’index existant n’a été réécrit ou supprimé.

## Résultat

Le dernier segment CONT-225, ajouté après le préfixe existant, se valide individuellement : 19/19 empreintes dans l’index du worktree et 20/20 dans l’index du projet; les deux préfixes conservent exactement leur compte de lignes et leur SHA-256 antérieurs. Le manifeste local du segment vérifie aussi son contenu.

La validation complète de chaque index échoue dans le snapshot courant. `sha256sum -c` trouve dans l’index du worktree 2 680 entrées dont le contenu au chemin actuel diffère de l’empreinte historique, 7 lignes mal formées et aucun chemin absent. Dans l’index projet, il trouve 1 958 empreintes différentes, 2 lignes mal formées et aucun chemin absent. Les nombres de lignes sont respectivement 13 358 et 7 672. Ces entrées historiques visent plusieurs versions de fichiers conservés sous le même chemin; cet audit ne permet pas d’attribuer chaque divergence à son changement d’origine. Les préfixes et empreintes n’ont pas été corrigés ni remplacés.

## Limites et suite

Le contrôle par `sha256sum -c` compare chaque empreinte historique au contenu courant d’un chemin mutable; après des mises à jour de code, documentation ou index lui-même, cette vérification complète ne peut pas distinguer un ancien snapshot attendu d’une modification non souhaitée. Les lignes mal formées doivent être auditées séparément avant toute décision sur la politique d’index. Le segment nouvellement ajouté est vérifiable, mais l’historique global n’est pas un manifeste entièrement reproductible au snapshot courant. La prochaine amélioration utile est un format d’archive versionné où chaque snapshot référencé reste immuable, en gardant les index existants intacts.
