# FORGEJO-COMMIT-STATUS-PAGINATION-001 — 26 octobre 2026

## Résultat

Le CLI `forgejo-reconcile-local --commit-status-snapshot <sha-complet>` lisait seulement `page=1`, donc un dépôt ayant plus de 50 contextes ne pouvait pas fournir un snapshot complet même si l’API permettait de lire les pages suivantes. L’adaptateur lit maintenant au plus 10 pages de 50 contextes (plafond 500), dans le même dépôt et pour le même SHA. Il vérifie que `X-Total-Count` reste identique, refuse les contextes répétés, et ne demande aucune page au-delà du plafond. Une pagination incohérente retourne `InconsistentPagination` avec une consigne actionnable de relire toutes les pages. Un total supérieur au plafond produit un snapshot partiel `inconclusive`.

Pour une lecture multipage, le résultat porte `snapshot_consistency=sequential_non_atomic`, `forgejo_reported_combined_state=null`, ainsi que les états rapportés pour chaque page. Cela représente les réponses observées, pas un état atomique du dépôt. Les capacités exposent la route avec `{page}`, les limites, la règle de refus en cas de changement de total et la cohérence non atomique.

La sortie reste du contexte consultatif. `independent_verification=false`, `merge_authorizing=false` et `current_pr_head_checked=false` restent inchangés : l’opération ne prouve pas que les checks ont réellement tourné, ne relit pas la tête actuelle d’une PR et ne fournit aucune autorité de fusion.

## Vérification

La première phase rouge est conservée dans `red.log` avec `red.exit=101`. Elle montre trois régressions ciblées avant implémentation : page suivante refusée, pagination multipage absente et dérive de total acceptée comme réponse partielle. Un second test rouge, conservé dans `duplicate-context-red.log` et `duplicate-context-red.exit`, a montré qu’une page unique contenant deux fois le même contexte était pourtant marquée `complete`; le parseur vérifie maintenant les doublons dans chaque page ainsi qu’entre pages. Après correction, le filtre `cargo test --locked --offline --bin forgejo-reconcile-local commit_status -- --test-threads=1` passe 10/10 dans `focused.log`. La suite complète de ce binaire passe 134/134 dans `bin-tests.log`, dont les tests qui lisent 51 contextes, refusent une variation `X-Total-Count` avec guidance, rejettent les doublons et s’arrêtent à 500 sans demander `page=11`.

`cargo fmt --all -- --check`, Clippy strict avec les quatre allowances historiques explicites du dépôt, `cargo run ... -- --capabilities`, `git diff --check`, et le contrôle de whitespace passent. `cargo metadata --locked --offline --no-deps` et le garde Cargo confirment `license=null`, `license_file=null`, `publish=[]`, `no_license_selected` et `redistribution_rights=not_assessed`. Le SHA local vérifie 26 artefacts. Les manifestes append-only produit et parent ont leurs empreintes courantes pour les 34 chemins requis et conservent leurs préfixes; les anciens snapshots de ces manifestes ne sont pas réécrits. Outils : Cargo/rustc 1.95.0. Les journaux et readbacks sont dans ce dossier.

Une première compilation pendant l’édition a détecté l’ambiguïté Rust d’un cast avant `<`; l’expression a été parenthésée avant les exécutions vertes. Aucun résultat de cette tentative n’est compté comme test réussi. La suite `--all-targets` n’a pas été relancée, puisque le changement est limité à ce binaire et que le worktree préexistant est très modifié.

## Sources et méthode

Forgejo v17.0.0 est fixé au commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`, étiquette exacte `v17.0.0`. L’examen ciblé couvre :

- `routers/api/v1/repo/status.go::GetCombinedCommitStatusByRef` : route, paramètres de page et en-tête `X-Total-Count`;
- `models/git/commit_status.go::GetLatestCommitStatus` : liste paginée des statuts les plus récents par contexte;
- `services/convert/status.go::ToCombinedStatus` : le `total_count` du corps reflète la page convertie, alors que l’en-tête porte le compte fourni par la requête paginée.

Les hashes SHA-256 des quatre fichiers amont, et leur égalité à `HEAD` dans le scratch `/tmp/forgejo-v17-base-cas-poc`, sont archivés dans `forgejo-source-sha256.txt`. Le scratch est globalement modifié; aucune source Forgejo n’a été copiée ou modifiée pour ce lot. L’examen du code amont n’est pas un test d’intégration live.

Les tests ajoutés utilisent un serveur HTTP loopback synthétique et les contrats du client Rust. Aucun serveur Forgejo n’a été lancé dans ce lot. Le rapport précédent avait hashé le source adaptateur à `a1aec58ea9a6b4936cad587b52475adffab5c337ff07a3d6cf0499436fb7d44f`; le hash après pagination et contrôle des doublons est `46ac7225240a31c246285f3bffe257b8877d8e85554aa51acde4d16bc0b13d44` (`current-adapter-sha256.txt`). Le worktree produit demeure à `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; il était déjà modifié avant ce lot et le diff global inclut ces modifications préexistantes. Aucun reset, commit, publication, accès externe, secret ou appel de modèle n’a eu lieu.

## Limites et suite

Les pages sont lues l’une après l’autre. Vérifier un total stable et des contextes distincts n’exclut pas qu’un statut ait changé entre deux lectures; le résultat n’est donc pas une photographie cohérente à un instant donné. Les statuts restent déclaratifs et potentiellement soumis par un écrivain du dépôt. La prochaine validation utile est un test de contrat contre une instance Forgejo locale jetable, si un environnement de test existant est disponible, sans réutiliser la décision comme preuve ni comme autorisation de fusion. La comparaison contrôlée avec agent et CI conventionnels et le runtime AgentOS restent des critères ouverts.
