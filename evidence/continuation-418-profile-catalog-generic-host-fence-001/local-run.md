# CONTINUATION-418 — profils partagés et refus du handoff générique

Date : 2026-10-28. Checkout local `local/WEBHOOK-005-canonical-links`; HEAD de référence `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Worktree déjà fortement modifié; ce lot n’a ni stage, commit, publication, push ou accès production.

## Résultat

Les deux copies de configuration de profil avaient divergé entre `forgejo-mission-local` et `mission-012-mcp-host`. `src/mission_profile.rs` est maintenant la source commune pour les profils locaux `duration-parser` et `mission-012-clamp`: origin, dépôt, branche, issue, exigence/chemin/révision, octets MISSION-012, workspace et capacités d’exécution. Le catalogue est compile-time host-owned et explicitement réservé aux adaptateurs loopback de test; il n’ouvre pas de sélection de profil agent ni d’autorité de fusion.

Le nouveau test `mcp_host_refuses_a_valid_generic_issue_41_handoff_before_lease_or_context` crée un vrai dépôt Git synthétique et les exigences de `agent-forge-core`/issue 41/`REQ-001`. Il appelle `admit_and_claim_mission_v2`, valide le DTO host-admission, puis donne ce DTO au vrai processus `mission-012-mcp-host`. L’hôte renvoie `mission_profile_scope_mismatch`, avec conseil de sélectionner un host pour le profil demandé, avant tout lease, reçu ou réponse MCP. Il n’y a aucun GET réseau: le fournisseur d’admission et le dépôt Git sont synthétiques. Ce test prouve le refus explicite du profil générique, pas son exécution.

## Vérifications exécutées

Toolchain observée : rustc 1.95.0, Cargo 1.95.0, Bubblewrap 0.9.0, Linux aarch64. Les compilations Rust utilisent `CARGO_TARGET_DIR=/dev/shm/debug` et l’option offline.

| Commande | Résultat final |
|---|---:|
| `cargo test --offline --locked --lib mission_profile::tests` | 2 réussis |
| `cargo test --offline --locked --bin forgejo-mission-local` | 48 réussis |
| `cargo test --offline --locked --bin mission-012-mcp-host` | 31 réussis |
| `cargo test --offline --locked --test mission_012_mcp_lifecycle` | 31 réussis, 1 ignoré |
| `rustfmt --edition 2024 --check` sur les cinq fichiers Rust touchés | passe |
| `git diff --check` | passe |
| `scripts/verify_cargo_license_state.py` et `scripts/test_cargo_license_state.py` | politique vérifiée; 12 tests réussis |

Les quatre journaux Cargo, leurs hashes de source/artifact et la sortie de metadata sont dans ce dossier. Aucun `all-targets` n’a été demandé pour ce changement ciblé; le test ignoré n’est pas compté comme réussite. Les processus de test ont tourné directement sous l’utilisateur local, sans Bubblewrap pour ce nouveau scénario; il n’utilise ni réseau ni code candidat.

## Portée et limites

- La composition MISSION-012 webhook → API d’issue courante → admission → `mission_context` est celle de CONT-360 et n’a pas été rejouée.
- Le test générique est construit par l’API d’admission de bibliothèque et une source synthétique, pas par une instance Forgejo ni par l’exécutable `forgejo-mission-local`.
- Le host demeure spécialisé MISSION-012; le catalogue commun prévient la dérive des pins, il ne le rend pas multi-profil.
- Pas de LLM, AgentOS, RAG, PR, vérificateur indépendant, merge ou comparaison de qualité dans ce lot.
- Le package reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`; les droits de redistribution restent non évalués.

## Accès GitLab PC4

La recherche autorisée dans le courriel a été limitée aux en-têtes/métadonnées. Elle a retrouvé des informations de compte root PC4 et un token destiné à un autre projet, sans credential limité à `root/gitspace-forgejo`; aucun corps ou valeur secrète n’a été lu ou utilisé pendant ce lot. Un contrôle anonyme confirme que le nom d’hôte ne se résout pas dans l’environnement de travail et que la connexion HTTPS au pair Tailscale expire. Aucun `ls-remote`, authentification, staging, commit, push ou transfert n’a eu lieu. Un ancien checkpoint signale que des valeurs avaient été exposées dans une sortie antérieure; traiter ces valeurs comme compromises, les révoquer/faire tourner et ne pas les réutiliser.

La suite distante exige une route HTTPS depuis l’environnement Codex et un token limité au projet avec les permissions Git minimales; lire les refs avant de préparer une branche dédiée. Jusque-là, continuer uniquement les lots locaux autorisés.
