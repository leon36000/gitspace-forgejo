# MERGE-007 — budgets de lecture avant parsing des revues

Date : 2026-10-22. Travail exécuté dans le workspace local isolé, au `HEAD` préexistant `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun commit n’a été créé. Le dépôt contenait déjà de nombreux changements et fichiers non suivis; ils ont été préservés. Le binaire Forgejo et ses sources n’ont pas été modifiés.

## Changement

`src/forgejo.rs` ajoute `PullRequestReviewPageBodyV1<R: Read>` et `collect_pull_request_review_json_pages`. Le nouvel adaptateur noyau vérifie la portée, le numéro séquentiel et la stabilité de `total_count` avant de lire chaque corps. Il emploie `Read::take` avec un octet supplémentaire pour détecter un dépassement avant désérialisation, puis délègue au collecteur existant qui contrôle complétude, doublons, IDs, limites d’entrées et champs.

- Maximum par réponse : 1 MiB, cohérent avec `MAX_RESPONSE_BYTES` déjà défini dans `src/bin/forgejo-reconcile-local.rs` pour son chemin existant.
- Maximum cumulé sur toutes les pages : 16 MiB, choix de ressource local et non une limite documentée Forgejo.
- Erreurs explicites : lecture impossible, page trop volumineuse, snapshot cumulé trop volumineux et JSON invalide.
- Les champs JSON inconnus ne sont pas conservés par le DTO. Les octets entiers consommés, champs inconnus compris, sont néanmoins comptés dans les budgets.

Le test rouge initial échouait à compiler parce que ces constantes, DTO et fonction n’existaient pas encore (`merge-007-json-bounds-red.log`). Les tests finaux vérifient qu’une page de 1 MiB et un ensemble de 16 MiB passent, que la lecture s’arrête à budget+1 pour une page trop grande et pour l’ensemble paginé, qu’un JSON mal formé et une erreur de `Read` produisent des erreurs distinctes, qu’un mauvais scope est refusé sans lire le corps, et qu’une page JSON valide donne le snapshot attendu.

## Vérifications

- `env CARGO_TARGET_DIR=/tmp/merge-005-tests cargo test --offline --locked --test forgejo_merge_approval` : 27 réussis, 0 échec, 0 ignoré.
- `env CARGO_TARGET_DIR=/tmp/merge-005-tests cargo test --offline --locked --all-targets` : 25 exécutables, 238 réussis, 0 échec, 7 ignorés.
- `env CARGO_TARGET_DIR=/tmp/merge-005-tests cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` : succès avec trois catégories de lints préexistantes neutralisées.
- `env CARGO_TARGET_DIR=/tmp/merge-005-tests cargo build --offline --locked --release --all-targets` : succès.
- `cargo fmt --all -- --check` et `git diff --check` : succès.

Une revue indépendante read-only n’a identifié aucun défaut P1/P2. Elle a confirmé la borne N+1, le budget cumulé, l’ordre des validations et la couverture d’erreurs. Le cas d’acceptation exacte à la limite cumulée a été ajouté après cette revue et passe dans la suite ciblée et générale.

## Limites et suite

Le `Read` synchrone peut bloquer : le futur transport doit imposer des échéances et annulations. Pour éviter une allocation antérieure non bornée, il doit transmettre le flux à cette interface plutôt qu’un `Vec` déjà téléchargé. L’interface reçoit `scope`, numéro de page et total comme métadonnées de l’adaptateur; elle n’authentifie ni le serveur, ni les en-têtes, ni l’identité. La pagination de Forgejo n’est pas un instantané atomique. La fonction n’est pas encore appelée par un client Forgejo de revue; aucun appel privé live, credential, décision de merge ou fusion n’a été exécuté.

La prochaine étape utile reste de trouver un canal d’autorisation hôte existant et vérifiable; en son absence, ajouter uniquement des contrats read-only qui n’exigent pas de secrets. Avant tout merge, l’adaptateur devra authentifier et borner le flux, relire les revues et le head/base immédiatement avant l’opération, respecter un gate indépendant et une identité distincte, et tester annulation/interruption et concurrence.

Le manifeste [MERGE-007](merge-007-json-final-sha256.txt) couvre 54 sources, documents et journaux de cette tranche et de ses prérequis; `sha256sum -c evidence/merge-007-json-final-sha256.txt` passe sur l’état courant.
