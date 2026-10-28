# CONTINUATION-434 — rejet des noms de tag qui ne sont pas des refs Git

Date : 28 octobre 2026 (America/Montreal).

## Lacune et correction

Le readback Release vérifiait que `tag_name` était non vide, limité à 255 octets et sans caractères de contrôle. Il ne vérifiait pas la syntaxe d’un nom de ref Git avant de construire l’URL canonique. Avec le tag invalide `feature/../outside`, la fixture API pouvait donc faire accepter et projeter `/releases/tag/feature/../outside`; un client ou proxy qui normalise les segments pourrait interpréter ce chemin différemment.

Le test rouge confirme l’acceptation avant correction. `valid_tag_ref_name` applique les contraintes de ref aux segments de tag tout en préservant les `/` valides et les tags valides commençant par `-`; `valid_branch` garde séparément sa restriction de dash initial. Le reconciler refuse maintenant les valeurs invalides comme `InvalidCurrentStateField("tag_name")` avant de dériver une URL.

## Preuves

Le test de réconciliation couvre cinq tags invalides : `feature/../outside`, `feature/./outside`, `feature//outside`, `feature/.hidden` et `feature/release.lock`. Un corpus indépendant exécuté avec `git check-ref-format refs/tags/<name>` rejette ces cinq valeurs et accepte `feature/v1.0` et `-release-v1`. Le test garde également une assertion que le nom de tag à dash initial reste valide.

Résultats locaux : le test rouge échoue parce que l’ancien code retourne `Ok` pour le tag traversant; après correction, le test ciblé passe 1/1. La suite `webhook_reconcile_contract` passe 102/102, `forgejo_merge_readiness` passe 10/10, `cargo fmt --all -- --check` et `git diff --check` passent. Aucun package ni dépendance n’a été ajouté.

## Limites

Les réponses de l’API Forgejo sont des fixtures locales; aucun Forgejo live n’émet de ref invalide dans ce test. Le corpus couvre sept noms, pas toutes les versions/forks, les règles de chaque client HTTP/proxy ou une conformité exhaustive de la grammaire Unicode. Ce lot ne démontre ni LLM/AgentOS, ni fusion contrôlée, ni gain comparatif. Il demeure sur le worktree PC6 jusqu’à son readback GitLab.
