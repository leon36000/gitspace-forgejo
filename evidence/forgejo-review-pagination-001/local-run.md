# FORGEJO-REVIEW-PAGINATION-001

Date : 25 octobre 2026. Dépôt local `agent-forge-core`, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; le worktree est fortement modifié et contient des changements antérieurs non attribués à ce lot.

## Changement et preuve

L’adaptateur `forgejo_review_pages` dans `src/bin/forgejo-reconcile-local.rs` transformait auparavant l’erreur `MergeApprovalError::IncompleteReviewList` en `ApiReadError::InvalidResponse`. La réponse au préflight de fusion masquait donc une pagination qui changeait pendant la lecture comme `upstream_invalid_response`, sans action ciblée.

Le test `tests::forgejo_review_page_adapter_reports_changed_total_as_retryable_inconclusive` sert une première page de 50 revues avec `X-Total-Count: 51`, puis la seconde avec un total de 52. Avant le changement, le test échoue sur le code générique. Après le changement, l’adaptateur renvoie `ApiReadError::InconsistentPagination`; l’autorité rend `inconclusive`, `review_pagination_inconsistent`, `retryable=true`, indique de jeter toutes les pages et de relire la liste complète, et laisse `merge_attempted=false` et `merge_authorized=false`. Le test vérifie les deux GET et leurs chemins exacts.

Les autres erreurs de collecte restent `InvalidResponse`. Le code d’erreur webhook générique est `upstream_pagination_inconsistent`; le code du préflight de merge et du gate est `review_pagination_inconsistent`. Cette erreur n’est pas considérée transitoire par la boucle de retry d’une lecture déjà mise en cache : l’action est une nouvelle lecture complète, sans réutiliser les pages partielles.

Une composition supplémentaire traverse `merge_authority_decision` sur la même fixture avec des totaux 51 puis 52. Elle confirme le stage `review-list-read`, le verdict `inconclusive`, l’absence d’autorisation et de tentative, les deux GET et aucun POST `/merge`. Une mutation temporaire du mapping vers `upstream_invalid_response` fait échouer ce test sur le code d’erreur; le mapping voulu est restauré avant la suite verte. L’extension de la fixture a d’abord changé l’ID 17 utilisé par les scénarios positifs et fait échouer cinq tests de gate; ce journal est conservé dans `all-targets-fixture-id-red.log`. La fixture garde maintenant l’ID 17 pour le cas singleton standard; la suite binaire et la suite complète passent.

## Commandes et résultats

- Rouge : `cargo test --offline --locked --bin forgejo-reconcile-local forgejo_review_page_adapter_reports_changed_total_as_retryable_inconclusive -- --nocapture` — échec attendu sur l’ancienne valeur `upstream_invalid_response`.
- Vert ciblé : même commande après correction — 1 réussi, 0 échec.
- Composition gate : `cargo test --offline --locked --bin forgejo-reconcile-local merge_authority_refuses_changed_review_total_before_any_merge_post -- --nocapture` — 1 réussi; mutation temporaire du mapping — échec attendu sur la classification.
- Suite binaire : `cargo test --offline --locked --bin forgejo-reconcile-local -- --test-threads=1` — 107 réussis, 0 échec.
- Suite fraîche : `cargo test --offline --locked --all-targets -- --test-threads=1` — 453 réussis, 0 échec, 30 ignorés, 34 groupes de test.
- Build : `cargo build --offline --locked --release --bin forgejo-reconcile-local` — réussi.
- Clippy : `cargo clippy --offline --locked --bin forgejo-reconcile-local -- -D warnings -A clippy::too_many_arguments -A clippy::collapsible_if -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` — réussi avec les quatre allowances explicites.
- Format et whitespace : `cargo fmt --all -- --check` et `git diff --check` — réussis après formatage.
- État de licence : `python3 -m unittest discover -s scripts -p test_cargo_license_state.py` — 7/7; le garde et le readback Cargo confirment `license=null`, `license_file=null`, `publish=[]`, `decision=unselected`, `redistribution_rights=not_assessed`. Aucune licence n’a été choisie.

Les logs bruts, y compris l’essai rouge du test de gate et le premier run rouge de la suite avec fixture incompatible, la sortie JSON du garde et le readback Cargo sont conservés dans ce dossier. Les versions de l’environnement et empreintes des sources sont consignées dans `toolchain.txt` et `source-sha256.txt`; `SHA256SUMS` couvre les artefacts du dossier.

## Portée et limites

La pagination est une fixture HTTP loopback; aucun serveur Forgejo réel, pull request ou merge live n’a été exercé. Le changement couvre l’adaptateur, la classification du résultat et le refus d’autorité; il ne constitue pas une composition live de toutes les lectures de la gate. Les tests purs `tests/forgejo_merge_approval.rs` couvrent déjà lacunes, doublons et incohérences de comptes; ce lot ajoute la variation de total au niveau HTTP de l’adaptateur.

La recherche en lecture seule des interfaces présentes dans `src/` et `Cargo.toml` n’a trouvé aucun client de modèle ni intégration locale AgentOS, Forge Code, RAG ou mémoire; elle n’a consulté aucun secret, `.claude`, environnement de session ou journal. Il n’y a donc pas de parcours LLM dans ce lot. Aucun appel externe, nouveau coût, accès ou déploiement n’a été utilisé.

Étape suivante : reprendre le host après arrêt en reconstruisant la candidate depuis la base admise et en ne rejouant qu’un intent durable exactement lié au même input et au hash avant/après observé; tester les états divergents et les points SIGKILL avant/après effet. Le test composé de pagination a maintenant établi le refus sans POST; Forgejo 046–050 ne sont pas rejoués ici.
