# CONTINUATION-132 — refus de write candidat via liens symboliques

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le worktree était fortement modifié; le fichier host `src/mission_012_host.rs` est non suivi dans ce dépôt. Rien n’a été staged, commité, supprimé, publié ou remis à zéro.

## Lacune et comportement

L’audit du write path MISSION-012 a retrouvé une protection OS dans `src/mission_012_host.rs::write_source`: ouverture du répertoire candidat et de `src` avec `openat(..., O_NOFOLLOW)`, puis contrôle du fichier via `fstatat(..., AT_SYMLINK_NOFOLLOW)` et exigence d’un inode régulier sans hardlink. Le test existant ne couvrait que les hardlinks. Deux variantes de symlink étaient donc sans preuve directe.

`host::tests::symlinked_candidate_source_or_source_directory_is_refused_without_touching_target` couvre le primitive :

1. `candidate/src/lib.rs` est un symlink vers un fichier frère du workspace;
2. `candidate/src` est un symlink vers un répertoire frère contenant `lib.rs`.

Dans les deux cas `write_source` retourne `tool_workspace_invalid`; le lien n’est pas remplacé et les octets de sa cible restent identiques. Le test n’essaie pas d’écrire dans cette cible. Le contrôle hardlink existant n’a pas été rejoué.

## Vérifications

- Test ajouté, ciblé : 1 réussi, 0 échec, 27 filtrés.
- Target complet `cargo test --offline --locked --bin mission-012-mcp-host` : 28 réussis, 0 échec, 0 ignoré.
- `cargo fmt --all -- --check` et `git diff --check` : code de sortie 0.

Une première recherche par filtre a visé `--lib`, qui n’inclut pas cette source; elle a exécuté 0 test et n’est pas comptée. Le test est dans le binaire `mission-012-mcp-host` sous le module `host::tests`; `-- --list` a établi le nom complet avant les runs ciblé puis complet. Les logs, commandes, versions, état Git et hashes sont dans ce dossier.

## Portée et limites

Cette preuve est unitaire et Linux locale sur la primitive host, pas un scénario MCP lancé pendant lequel un autre processus remplace les chemins, ni une course TOCTOU, un test multi-UID ou une séparation contre un acteur privilégié. Elle ne démontre ni Forgejo live, LLM/AgentOS, ni gain comparatif. Aucun code runtime n’a changé. Les limites du lanceur et de l’isolation restent celles décrites dans l’architecture; un worktree à lui seul ne sépare pas les privilèges d’exécution.

Projet toujours incomplet. La suite doit poursuivre une lacune distincte; aucune nouvelle dépense ou intégration externe n’est impliquée par ce lot.
