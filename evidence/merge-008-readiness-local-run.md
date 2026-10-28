# MERGE-008 — verdict conjoint proof + approbations

## Changement

`src/forgejo.rs::evaluate_merge_readiness` compose la préflight du proof signé avec l’évaluation d’un snapshot de revues complet. Le résultat lie origine, dépôt, PR, SHA de base et de tête, hashes des exigences et de la politique, identité du vérificateur, empreinte des revues et IDs des approbations/reviewers. Le candidat évalué est le head du snapshot PR, et la portée des revues est construite depuis le scope de branche validé et le numéro de PR attendu.

La relecture indépendante a relevé un mismatch entre le vérificateur lié au proof et `Policy.verifier_id`. Le test `policy_and_passing_proof_must_name_the_same_verifier` a d’abord échoué parce que le composite acceptait ces identités différentes. `MergeApprovalDecisionV1` retourne maintenant l’identité déclarée par la politique validée; `evaluate_merge_readiness` refuse la divergence par `MergeApprovalError::VerifierMismatch`.

## Vérifications

- Test ciblé : `cargo test --offline --locked --test forgejo_merge_readiness` — 5 réussis, 0 échec.
- Suite : `cargo test --offline --locked --all-targets` — 26 exécutables, 243 réussis, 0 échec, 7 ignorés.
- Clippy : `cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` — réussi avec les trois allowances déjà utilisées par le projet.
- Build : `cargo build --offline --locked --release --all-targets` — réussi.
- Formatage : `cargo fmt --all -- --check` — réussi.
- Diff : `git diff --check` — réussi; contrôle des fins LF et espaces de fin — réussi sur les sources et documents du lot.
- Revue indépendante read-only : un P2 trouvé puis corrigé; la seconde revue ne trouve aucun P1/P2. Hash examinés : `src/forgejo.rs` `501ad03ff9c9a1496863617e3df3c26f7d29720c00211b1176ee96dac3ed8799`; `tests/forgejo_merge_readiness.rs` `5f8b1490dd1ace306772a926c40bc735e113d9090462ec7d326e3f21341aaab0`.

Journaux : `merge-008-readiness-red.log`, `merge-008-policy-verifier-red.log`, `merge-008-readiness-focused.log`, `merge-008-readiness-all-targets.log`, `merge-008-readiness-clippy.log`, `merge-008-readiness-release-build.log`, `merge-008-readiness-rustfmt-apply.log`, `merge-008-readiness-rustfmt.log`, `merge-008-readiness-diff-check.log`.

## Limites

Le code est un contrat local sans appelant de production. Il suppose que l’hôte authentifie les sources et lie `execution_reviewer_id` à `context.execution_identity`; il ne garantit pas la provenance, l’atomicité de la pagination, une relecture finale sous verrou ou l’autorité de fusion. Pas d’API Forgejo privée/authentifiée, d’agent LLM, de décision humaine ou de fusion exercée. Le résultat ne comporte pas de permission de fusion.

Suite utile : construire l’adaptateur de lecture hôte en lecture seule, avec scope/origine et réponses HTTP authentifiés, limites de flux, délais et annulation. Tant qu’aucune source hôte autorisée n’est constatée, rester sur fixtures et ne créer aucun credential.
