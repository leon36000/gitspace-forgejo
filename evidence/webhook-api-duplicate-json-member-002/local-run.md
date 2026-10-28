# WEBHOOK-API-DUPLICATE-JSON-MEMBER-002 — décodage strict des autres réponses API Forgejo

Date : 25 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, déjà fortement modifié et sans commit dédié.

## Constats et changement

L’audit ciblé a suivi les réponses JSON des lecteurs locaux d’admission et de fusion. `LocalLoopbackForgejoApi::request_api_once` utilisait déjà le parseur strict du lot 001. Deux lecteurs distincts ne le partageaient pas : `LocalLoopbackForgejoSource::json_get` analysait les réponses d’issue, de branche, de paramètres et d’arbre comme `serde_json::Value`; `collect_pull_request_review_json_pages` décodait directement le JSON d’une page en DTO.

Deux rouges sont archivés :

- Une fixture HTTP loopback a répondu au vrai binaire mission avec `{"name":"main","protected":false,"protected":true,"commit":{"id":"f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0"}}`. L’ancien chemin renvoyait `Ok(ProtectedBranchHeadV1 { protected: true, ... })`; le test exigeait `InvalidResponse`. Voir `test-red.log` (exit 101).
- La page de revue contenait deux clés inconnues `extension`. La désérialisation en DTO les ignorait et acceptait la revue; le test exigeait `InvalidReviewResponse`. Voir `review-pages-red.log` (exit 101).

Les deux lecteurs passent maintenant les octets à `src/strict_json.rs::parse_value_rejecting_duplicate_members`, puis effectuent leur validation métier ou la conversion vers le DTO. La comparaison porte sur les clés après décodage des échappements et s’applique aussi aux objets imbriqués. Les tests ciblés verts couvrent l’admission de branche par HTTP/curl local et le collecteur public de pages de revue.

## Vérifications

- Test rouge puis vert du lecteur mission : 1 test ciblé; le test final exige `Err(SourceFailureV1::InvalidResponse)` après avoir servi une seule réponse locale.
- Test rouge puis vert du collecteur de revues : 1 test ciblé; le résultat final exige `Err(MergeApprovalError::InvalidReviewResponse)`.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : 492 réussis, 0 échec, 55 ignorés, 34 groupes; code de sortie 0. Le run final est `all-targets-final.log`.
- `cargo fmt --all -- --check` : code 0 après formatage ciblé de `src/forgejo.rs`.
- `git diff --check` : code 0.
- `cargo clippy --offline --locked --lib -- -D warnings -A clippy::too_many_arguments -A clippy::collapsible_if` : code 0 avec ces deux allowances historiques.
- `python3 scripts/verify_cargo_license_state.py` : `verified`; aucune licence choisie ni changement Cargo.

Les commandes, versions, logs, codes de sortie, empreintes de sources et manifeste sont conservés dans ce dossier. Le manifeste local couvre seulement les fichiers de preuve du présent lot; il est vérifié séparément.

## Portée et limites

Le test de branche lance le vrai `curl` du binaire vers une fixture HTTP liée à `127.0.0.1` et à un port éphémère; il ne contacte pas Forgejo. Le test de revue fournit des octets synthétiques au collecteur de corps borné; il ne prouve pas la cohérence des pages d’un serveur vivant. Les réponses HTTP d’admission de ce binaire restent une source de test locale et non authentifiée : ce changement rejette les corps JSON ambigus mais n’authentifie pas Forgejo. Les fichiers bruts `raw/commit/...`, notamment les exigences, et leurs parseurs sont hors de cette tranche. Le surcoût mémoire du passage transitoire par `Value` n’a pas été mesuré; les lecteurs API existants plafonnent déjà les réponses à 1 MiB et le collecteur de revues à 1 MiB par page.

Cette tranche ne démontre ni un parcours avec LLM, AgentOS, Forge Code, RAG ou mémoire, ni une comparaison contrôlée de coût/qualité. Aucun service externe, secret utilisateur, coût, publication, modification de production ou code Forgejo/GitSpace n’a été utilisé.
