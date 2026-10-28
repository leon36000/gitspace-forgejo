# WEBHOOK-REVIEW-COUNT-HEADER-DUPLICATE-001 — refuser les comptes de pagination ambigus

Date : 25 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, sans commit dédié et déjà très modifié.

## Reproduction et correctif

La pagination des revues dépend de l’en-tête `X-Total-Count`. `build_curl_api_request` demandait `%header{x-total-count}` au `--write-out` de curl. Le test rouge a servi une réponse HTTP 200 loopback avec `X-Total-Count: 1` puis `X-Total-Count: 2`; le vrai `LocalLoopbackForgejoApi::request_api_once` a rendu `Ok(Some(1))`, perdant la valeur contradictoire. Le résultat rouge est conservé dans `test-red.log` (exit 101).

Le client capture maintenant `%{header_json}`, vérifie cette structure avec le parseur JSON strict, puis lit les valeurs d’en-tête comme tableaux. Un `X-Total-Count` répété fait échouer la réponse `InvalidResponse`; une valeur absente reste `None`, et une valeur unique doit être un entier non signé. Un corps reste limité à 1 MiB et les métadonnées d’en-tête JSON à 64 KiB. Une réponse contenant plusieurs `Retry-After` donne un délai invalide, ce qui empêche le retry. Les capacités versionnées v1 publient les limites et les deux comportements.

## Résultats vérifiés

- Le test vert `local_adapter_rejects_duplicate_total_count_headers` refuse le même en-tête contradictoire avec `InvalidResponse`; le fixture sert exactement une requête.
- `cargo test --offline --locked --bin forgejo-reconcile-local -- --test-threads=1` : 113 réussis, 0 échec, 0 ignoré.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : 493 réussis, 0 échec, 55 ignorés sur 34 groupes; code de sortie 0.
- `cargo fmt --all -- --check` : code 0.
- `cargo clippy --offline --locked --bin forgejo-reconcile-local -- -D warnings -A clippy::too_many_arguments -A clippy::collapsible_if` : code 0.
- Le même Clippy sans allowances échoue sur dix erreurs `too_many_arguments`/`collapsible_if` dans les modules voisins préexistants (`agent_runtime.rs`, `forgejo.rs`, `mission_admission.rs`, `lib.rs`); aucune erreur Clippy ne reste dans ce binaire avec les deux catégories historiques autorisées.
- `python3 scripts/verify_cargo_license_state.py` : `verified`, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.

Les sorties de commandes, codes, versions, empreintes des sources et manifestes sont dans ce dossier.

## Portée et limites

Le test emploie le vrai processus `/usr/bin/curl` 8.5.0 vers un serveur HTTP synthétique sur loopback; il ne lance pas Forgejo. Il montre la perte d’une valeur d’en-tête et le refus par le lecteur central, mais ne démontre pas à lui seul qu’un run complet aurait fusionné une PR à tort. Le test ne traverse pas l’autorité de fusion complète et n’envoie aucun POST de merge. Le minimum de version curl nécessaire pour `%{header_json}` n’a pas été établi; seule la version installée 8.5.0 a été vérifiée ici.

Le changement ne choisit aucune licence et ne modifie pas Cargo. Il n’utilise ni Forgejo/GitSpace live, ni agent LLM, AgentOS, RAG, donnée privée, service externe, dépense, publication ou production. Le parcours LLM et le benchmark contrôlé restent non démontrés.
