# MERGE-010 — lier chaque revue à l’URL de sa PR

**Date : 22 octobre 2026.** Lot Rust local sur le worktree existant. Contrat et fixtures uniquement : aucun lecteur de revues Forgejo n’a été raccordé, aucune requête authentifiée, identité humaine, modification upstream ou fusion n’a eu lieu.

## Sources vérifiées

Forgejo officiel, clone détaché `/tmp/forgejo-audit-merge006`, commit exact `ea9ae1e174ac290febcd92f57e7a28ccb58262d0` :

- `modules/structs/pull_review.go:29–46` déclare le JSON `pull_request_url` sur `PullReview`.
- `services/convert/pull_review.go:24–38` remplit `HTMLPullURL` par `r.Issue.HTMLURL()`.
- `models/issues/issue.go:404–412` forme cette URL depuis `repo.HTMLURL()`, `pulls` et l’index.
- `models/repo/repo.go:344–345` forme `repo.HTMLURL()` avec `url.PathEscape(owner)` et `url.PathEscape(repo)`.

Comportement d’encodage comparé à la source officielle Go [`net/url.PathEscape`](https://go.dev/src/net/url/url.go#L189-L192) et sa table [encodePathSegment](https://go.dev/src/net/url/gen_encoding_table.go#L286-L305). Go n’est pas installé sur cet hôte (`go: command not found`); l’équivalence est issue de la lecture du code source Go, pas d’une exécution Go ou Forgejo. Le test Rust couvre la composition avec préfixe de déploiement pour des noms Forgejo valides. Ce lot n’importe ni ne copie le code de Forgejo ou de Go.

## Contrat implémenté

`PullRequestReviewV1` exige maintenant `pull_request_url` à la désérialisation. Le collecteur de pages et `validate_merge_approvals` le comparent tous deux à l’URL construite à partir de la portée hôte (origine, préfixe de déploiement, propriétaire, dépôt et numéro). Les segments propriétaire/dépôt suivent le traitement Go `PathEscape` observé dans Forgejo; le champ URL est limité à 4096 octets. Une URL incorrecte ou absente échoue fermé. Le fingerprint a évolué vers `CanonicalPullRequestReviewsV2` (`schema_version=2`) pour inclure ce champ dans la projection consommée.

Le test rouge `review_from_another_pull_request_is_rejected_by_its_url` a d’abord reçu `Ok(PullRequestReviewsSnapshotV1)` pour une réponse portant `/pulls/24` quand le scope demandait `/pulls/23`, alors qu’il attendait `InvalidReviewSet` (code 101). Trace : `evidence/merge-010-review-url-red.log`. Après correctif :

- `cargo test --offline --locked --test forgejo_merge_approval` : 32 réussis.
- `cargo test --offline --locked --test forgejo_merge_readiness` : 10 réussis.
- Les tests vérifient URL manquante, PR d’URL différente aux entrées collecte et validation directe, URL à 4096 octets acceptée/4097 refusée, et composition avec le préfixe `/forgejo/` et des noms Forgejo valides.
- Le fingerprint canonique v2 est `83a3f6713e1e53d0ee44a5deb3cb7f3a80223436db19880f0d06f0a4d31b5b89`. Python 3 a recomposé le JSON compact à champs ordonnés et en a recalculé le SHA-256 indépendamment.

## Vérifications finales

- `CARGO_TARGET_DIR=/tmp/merge-010-tests cargo test --offline --locked --all-targets` : 26 exécutables, 253 réussis, 0 échec, 7 ignorés. Les ignorés ne sont pas comptés comme passés.
- Clippy `--all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` : code 0 avec les trois allowances préexistantes du dépôt.
- `cargo build --offline --locked --release --all-targets`, `cargo fmt --all -- --check` et `git diff --check` : code 0.
- Contrôle Python ciblé des sources Rust nouvellement modifiées : aucune espace finale ni NUL.

Journaux : `merge-010-review-url-{red,focused,all-targets,clippy,release-build,rustfmt,diff-check}.log` et `merge-010-readiness-focused.log`. Le log all-targets agrège 26 sections `test result`. Les sources, tests, synthèses et journaux de ce lot sont listés dans `evidence/merge-010-review-url-final-sha256.txt`.

## Portée, examen indépendant et limites

L’URL est une valeur déclarée dans le JSON que le futur lecteur reçoit. La comparaison d’URL protège le contrat contre un mélange de PR et détecte une réponse incohérente; elle ne prouve pas qui a produit la réponse ni quelle identité HTTP l’a lue. Les pages restent non atomiques. Aucune provenance d’approbation/base n’est fournie par Forgejo : l’attestation `review_id → base_branch/base_commit` de MERGE-009 reste une entrée hôte non authentifiée, sans producteur fiable. Aucun appelant de production, merge gate, relecture sous exclusion, POST ou résultat de fusion n’existe.

Le reviewer indépendant n’a trouvé aucun P1/P2. Il a d’abord identifié une fixture P3 avec des noms rejetés par Forgejo; la fixture d’URL emploie maintenant `team-unit`/`repo_v2`, et `valid_segment` refuse les caractères non path-safe `+` et `#`. Le second examen confirme que cette réserve est fermée. Le dernier état vérifié ici est un contrat Rust compilé et testé contre des fixtures, pas une intégration Forgejo en bout en bout.
