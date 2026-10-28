# CONTINUATION-293 — relecture bornée des statuts multi-pages

Date d’exécution : 27 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre de travail fortement modifié, aucun commit créé. Les manifestes identifient le worktree et les artefacts sans prétendre que cet arbre sale est un checkout propre.

## Lacune et changement

Dans `src/bin/forgejo-reconcile-local.rs`, `LocalLoopbackForgejoApi::read_commit_status_snapshot` agrège les pages de `/commits/{sha}/status`. Avant ce lot, un total `X-Total-Count` stable et des contextes uniques suffisaient à retourner `observation=complete`, même si un statut avait changé entre les pages.

Pour un résultat multi-page complet, le reconciler relit maintenant séquentiellement chaque page une fois. Il compare le total, les statuts normalisés de la page et l’état agrégé que Forgejo rapporte. Toute différence rend `ApiReadError::InconsistentPagination` et demande au client de relire le snapshot; la lecture s’arrête à la première page divergente. Les réponses tronquées restent inconclusives et ne passent pas par cette seconde passe. Les lectures suivent le même budget, la même origine autorisée et les mêmes retries bornés que les lectures existantes.

La découverte des capacités passe de schéma/API v16 à v17 et décrit la relecture, avec au plus 20 lectures de pages pour un résultat complet multi-page si aucun retry n’est nécessaire. Le schéma des objets snapshot reste v1 et `snapshot_consistency` reste `sequential_non_atomic`. La comparaison n’est ni un instantané atomique ni une vérification indépendante; les champs `independent_verification` et `merge_authorizing` restent `false`.

## Test rouge, correction et mutation

Le premier test loopback à 51 statuts a été écrit et lancé avant le code de relecture. Il gardait `X-Total-Count=51`, servait 50 contextes puis un, et changeait `ci/0` entre les passages. Le code antérieur renvoyait `complete`; `red.log` conserve l’échec de l’assertion `unwrap_err` sur ce résultat positif.

La fixture finale est plus discriminante et couvre la seconde page : 100 statuts sur deux pages, total `X-Total-Count=100`. La première page est inchangée au recontrôle. La deuxième contient `ci/51=failure`, et `ci/50` change de `success` à `pending`; le compte, les contextes et l’état agrégé `failure` de cette page restent identiques. Le test `commit_status_snapshot_rejects_constant_total_status_drift_between_pages` exige le refus après quatre GET — pages 1 et 2 du premier passage, puis pages 1 et 2 du recontrôle.

Un test de mutation temporaire a retiré uniquement la comparaison des statuts individuels, en laissant la comparaison de l’état agrégé active. Avec la fixture finale de 100 statuts, le test discriminant a alors échoué en recevant `complete`; `status-comparison-mutation-final.log` le conserve. La comparaison est restaurée dans la source finale. `status-comparison-mutation.log` conserve aussi une mutation précédente avec une fixture de 51 statuts. Les itérations initiales sont gardées : `focused.log` expose une erreur de compilation d’indice `u64` corrigée; `focused-rerun.log` montre l’assertion initiale erronée de quatre requêtes (le refus attendu s’arrête à la troisième); `focused-final.log` passe avec la première fixture.

## Vérifications finales

- `cargo test --locked --offline --bin forgejo-reconcile-local commit_status_snapshot` : 9 réussis, 0 échec (`pagination-page2-final.log`).
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e` : 5 réussis, 0 échec, 2 ignorés (les tests de routes liées au port fixe, à lancer explicitement et sériellement).
- `cargo test --locked --offline --all-targets` : **695 réussis, 0 échec, 154 ignorés, 37 suites** (`all-targets-page2-final.log`). Les tests annotés `ignore` ne sont pas exécutés par cette commande.
- `cargo fmt --all -- --check`, `rustfmt --check --edition 2024 --config skip_children=true src/bin/forgejo-reconcile-local.rs tests/forgejo_reconcile_capabilities_e2e.rs`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` : exit 0.
- Le guard de licence rapporte `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`; ce résultat ne tranche pas les droits de redistribution.

Outils : Rust/rustfmt 1.95.0, Cargo 1.95.0, Git 2.43.0, Python 3.12.3, Linux/aarch64. Les versions, commandes, reçus de tests et empreintes sont conservés dans ce dossier.

## Portée et limites

Tous les cas de comportement sont exercés par un serveur HTTP loopback contrôlé par les tests; aucune instance Forgejo live ni runner Actions n’est utilisé. Le test couvre un changement de statut dans la première page d’un résultat complet de deux pages, avec total et état agrégé stables. Le code compare toutes les pages et ces deux champs, mais les autres variantes ne sont pas des échantillons répétés. Les deux passes restent séquentielles : une mutation transitoire qui se produit et revient entre les observations peut échapper au contrôle; aucun verrou ne bloque les writers Forgejo externes. Jusqu’à 20 lectures de page sans retry et un délai d’adaptateur plus long sont possibles pour un snapshot complet; aucune latence, charge ou dépense réelle n’est mesurée.

Ce lot réduit le risque qu’un changement observable dans une page soit présenté comme une lecture complète. Il ne crée pas de preuve d’acceptation indépendante, ne transforme pas l’état Forgejo en verdict de CI et ne permet pas une fusion. Il ne démontre pas AgentOS, un parcours LLM ou un gain comparatif. Les critères cumulatifs de la mission restent incomplets.
