# MERGE-009 — lier les approbations à leur base

**Date : 22 octobre 2026.** Contrat Rust et tests locaux; audit read-only du code Forgejo officiel. Aucune modification de Forgejo, requête authentifiée, décision humaine ni fusion.

## Motif et source

L’audit précédent établissait que la route de merge compare le head attendu, sans compare-and-swap sur le base SHA. La relecture complémentaire du dépôt officiel au commit exact `ea9ae1e174ac290febcd92f57e7a28ccb58262d0` révèle aussi qu’une approbation restée valide au même head peut ne plus correspondre à la base examinée :

- `services/pull/pull.go::ChangeTargetBranch` (lignes 167–263) retourne sans événement si la base est déjà la cible (171–173); sinon la base et le diff sont enregistrés avant que le commentaire `CommentTypeChangeTargetBranch` ne soit créé (246–260). Si la création du commentaire échoue, la base a changé et l’appelant n’émet pas la notification qui suit le succès (`routers/api/v1/repo/pull.go:767–787`).
- `services/pull/pull.go::TestPullRequest` invalide les revues dans le chemin de synchronisation de la tête si le diff change (333–348, 375–412). Le chemin de push sur la base met à jour divergence et queue (351–372), sans marquer les revues périmées.
- La conversion REST expose `commit_id`, `stale`, `official`, `dismissed` et l’URL de la PR (lignes 24–38 de `services/convert/pull_review.go`). Le DTO local garde les premiers champs mais pas `pull_request_url`; le futur adaptateur devra comparer aussi cette URL à la PR demandée.
- Le handler de revue compte les résultats et convertit la page (`routers/api/v1/repo/pull_review.go:84–108`). `ToPullReviewList` retire des réponses les revues `PENDING` que l’appelant ne peut pas voir (`services/convert/pull_review.go:64–78`); le total peut donc dépasser les lignes visibles. Notre collecteur échoue fermé sur ce cas, ce qui peut refuser une décision, pas créer une approbation.
- La timeline filtre des événements, commentaires de code et xrefs accessibles, puis donne à `X-Total-Count` le nombre de lignes rendues pour la page courante (`routers/api/v1/repo/issue_comment.go:189–223`). Une lecture de timeline ne prouve pas qu’aucun événement de changement de base n’a été omis.

Ces constatations ont été contrôlées dans le clone officiel local `/tmp/forgejo-audit-merge006`, checkout détaché et propre, via `nl -ba` et lecture des chemins listés; aucun test ou build Go n’a été exécuté. Les références de commit sont regroupées dans `evidence/merge-004-source-audit.md` et les liens Codeberg y pointent vers les lignes auditées.

## Contrat implémenté

`src/forgejo.rs` définit `PullRequestReviewBaseBindingV1` (ID de revue, branche et SHA de base) et `PullRequestReviewBaseBindingsSnapshotV1` (scope PR et liaisons). `evaluate_merge_readiness` exige une liaison par approbation retenue; il refuse l’absence, les IDs nuls/dupliqués/inattendus, une portée incorrecte, un hash ou ref de base invalide et une base qui ne correspond plus au snapshot courant. Il trie les liaisons par ID pour leur SHA-256 canonique et ajoute `base_branch` et `review_base_bindings_sha256` au résultat read-only.

Le test rouge `approval_without_a_trusted_base_binding_is_not_ready` a été exécuté en omettant temporairement l’appel de validation : il retourne `Ok(MergeReadinessDecisionV1)` sans binding alors que le test attend `MissingReviewBaseBinding` (code 101). Le script a restauré `src/forgejo.rs` puis vérifié l’égalité byte-à-byte par SHA-256. Journal : `evidence/merge-009-base-binding-red.log`.

Dix tests focaux passent sous `CARGO_TARGET_DIR=/tmp/merge-009-tests` : readiness positive, binding manquant, autre branche ou SHA de base, autre PR, doublon, preuve au head périmé, autre PR d’approbation, ancienne approbation, mismatch d’identité du vérificateur et empreinte de bindings. Ce dernier test vérifie l’invariance à l’ordre, l’effet d’un changement de base et le vecteur `405838373b3ee0f03e008bc25b03791f0fc0a54d69880fbe5ba49b12abb0aad7`, calculé indépendamment avec `hashlib.sha256` sur JSON compact. Sortie : `evidence/merge-009-base-binding-focused.log`.

## Vérifications finales

- `CARGO_TARGET_DIR=/tmp/merge-009-tests cargo test --offline --locked --all-targets` : 26 exécutables de test, 248 réussis, 0 échec et 7 ignorés. Aucun test subprocess marqué `ignored` n’est compté comme réussi.
- Clippy all-targets passe avec `-D warnings` et les trois exceptions historiques du dépôt : `too_many_arguments`, `bool_assert_comparison`, `manual_div_ceil`.
- Le build release all-targets passe; `cargo fmt --all -- --check` et `git diff --check` retournent 0.
- Revue indépendante en lecture seule après ajout du vecteur : le test correspond au hash recalculé par Python, et la documentation indique bien la limite de provenance. Aucun autre défaut matériel n’est signalé. Le reviewer confirme que le binding reste une entrée de confiance, non une attestation prouvée.

Journaux finaux : `evidence/merge-009-base-binding-{focused,all-targets,clippy,release-build,rustfmt,diff-check}.log`. Le test du vecteur a d’abord échoué sur une valeur provisoire, puis le vecteur calculé indépendamment a été fixé; la trace intermédiaire est `evidence/merge-009-base-binding-fingerprint-red.log`.

## Portée et limite

Le type public ne signe ni n’authentifie l’attestation. Le helper de fixture construit les liaisons depuis le snapshot courant pour tester les branches positives/négatives; il ne constitue pas une preuve Forgejo. L’API auditée ne donne pas le SHA de base au moment de chaque approbation, et la timeline n’est pas exhaustive. Il manque donc encore un producteur fiable pour `review_id → base_branch/base_commit`.

Cette interface réduit le risque d’une future implémentation qui oublierait le binding, mais ne corrige pas le comportement Forgejo observé et n’autorise aucune fusion. Une signature/empreinte ne prouverait pas qu’un binding correspond à l’approbation. Avant toute autorité de merge, il faut établir une source fiable de ce lien (potentiellement un changement ciblé au point d’approbation/merge) et faire vérifier la PR, la base et la politique au moment même de la fusion. La vérification d’URL `pull_request_url`, le lecteur authentifié, les pages atomiques, l’identité réelle et la frontière d’autorisation restent hors de ce lot.

**Mise à jour historique :** MERGE-010 implémente ensuite la vérification locale de `pull_request_url` contre le scope demandé. Cela ne fournit toujours ni la source fiable du binding de base ni l’authenticité du transport; voir `evidence/merge-010-review-url-local-run.md`.
