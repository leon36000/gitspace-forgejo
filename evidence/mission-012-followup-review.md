# Revue indépendante de suivi — MISSION-012

Date : 2026-10-22. Revue en lecture seule par un sous-agent Codex; elle n’a modifié ni exécuté le dépôt.

La revue a ciblé le matériau ajouté pour matérialiser les sources vérifiées. Elle confirme que la collecte conserve chaque chemin frère, trie les répertoires du plus profond au moins profond avant d’appliquer les permissions, et déduplique uniquement les chemins identiques. Le test associé parcourt récursivement les modes des fichiers et répertoires du fixture multi-répertoires.

La revue ne prétend pas qu’un mode Unix read-only protège des mutations par un processus du même UID : cette limite reste explicitement documentée. Elle ne valide pas non plus la provenance Forgejo du commit; le vérificateur local reçoit encore des références Git locales et n’authentifie pas lui-même l’association commit/PR. Enfin, elle est statique : la validation dynamique est apportée séparément par la suite ci-dessous.

Le test de processus MCP inclut aussi une lecture explicite de `src/lib.rs` avant la commande de base; le test ciblé et la suite complète ont été relancés après cette modification.

## Vérifications fraîches

- `cargo test --offline --locked --test mission_012_mcp_lifecycle` : 1 réussi, 0 échec.
- `CARGO_TARGET_DIR=/tmp/mission012-main-final-target cargo test --offline --locked --all-targets` : 198 réussis, 0 échec, 2 ignorés.
- `cargo fmt --all -- --check` : code de sortie 0.
- `git diff --check` : code de sortie 0.
- Clippy ciblé pour les deux binaires MISSION-012 avec `-D warnings -A clippy::too_many_arguments` : code de sortie 0.

Les journaux exhaustifs correspondants sont dans ce répertoire `evidence/`.
