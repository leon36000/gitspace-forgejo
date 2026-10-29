# CONTINUATION-446 — readback PR partiel en réconciliation

Date : 28 octobre 2026. Audit source-only, sans code ou test produit modifié. Worktree de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, déjà fortement modifié.

## Question et décision

CONTINUATION-445 avait strictement lié les trois champs d’identité du dépôt base dans la **confirmation d’une fusion**. Le checkpoint demandait si le parseur de réconciliation lecture seule devait lui aussi refuser les readbacks où `base.repo` est absent, nul ou partiel.

La réconciliation ne transforme pas `base.repo` en champ de `ReconciledState`; elle utilise les champs fournis uniquement pour détecter une contradiction avec la cible liée à l’URL. Le contrat exposé par `--capabilities` et figé dans `tests/forgejo_reconcile_capabilities_e2e.rs` déclare explicitement : pour `base.repo`, quand l’objet est présent, valider chaque identité fournie; l’absence/null est permis comme réponse API partielle. Les tests de contrat couvrent nom seul, owner seul, null et absent, ainsi que les identités contradictoires. Le readback est descriptif et n’autorise pas une fusion.

Forgejo v17.0.0 confirme que `PRBranchInfo.Repository` est un pointeur. La route `GET /repos/{owner}/{repo}/pulls/{index}` charge d’abord le dépôt base, puis passe par `ToAPIPullRequest`; le convertisseur affecte `Repository: ToRepo(...)`. Dans `innerToRepo`, une erreur de `repo.GetBaseRepo(ctx)` retourne `nil`, qui est sérialisé comme `repo: null`. Il existe donc un chemin amont source-visible où les métadonnées d’identité peuvent être nulles sans invalider le reste du snapshot PR. Décision : conserver l’acceptation partielle dans la réconciliation descriptive, avec contrôle de toutes les valeurs présentes. Garder les gates de fusion stricts sur les trois champs complets et égaux.

## Références exactes

La source amont est Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; fichiers, lignes et blobs sont dans `upstream-source.txt`. Le contrat local se trouve dans `src/bin/forgejo-reconcile-local.rs` et `tests/forgejo_reconcile_capabilities_e2e.rs`; la fonction de validation descriptive est `src/webhook/reconcile.rs::validate_current_pull_request_base_repository`. L’identité n’est pas utilisée comme preuve de fusion; `src/bin/forgejo-reconcile-local.rs::merge_readback_is_confirmed` est séparément stricte.

## Portée et limites

Il s’agit d’une lecture ciblée de source et de contrat, sans serveur Forgejo live, sans comportement API observé, sans changement runtime et sans exécution de tests. Le code amont montre un chemin possible vers `repo: null`, pas la fréquence de ce cas ni le résultat exact des handlers pour tous les états DB. Les tests locaux vérifient le contrat explicite mais n’établissent pas que tout client ou version Forgejo renvoie ces formes. La politique partielle n’établit aucune identité absente et ne doit pas être réutilisée pour autoriser un merge.
