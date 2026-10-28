# WEBHOOK-MCP-CLAIM-004 — durcir l’hôte MCP claim-bound MISSION-012

Date de vérification : 2026-10-23 02:06:09 UTC
Checkout : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`
État : travail local non commité ; `src/bin/mission-012-mcp-host.rs` et `tests/mission_012_mcp_lifecycle.rs` sont des fichiers locaux non suivis par Git, et ce sont les deux seuls fichiers de code modifiés dans ce lot.

## Résultat

Les cinq findings LOW de durabilité et de déni de service relevés par la revue de `mission-012-mcp-host` sont corrigés :

- **Lease non consommé sur workspace invalide.** L’empreinte de la requête et `CandidateToolHost` sont construits avant `acquire_lease`. Un candidat qui n’est pas à la base est refusé (`candidate_not_at_base`, code 65) sans événement `mission_execution_lease_acquired_v1`. Le même ID de lease reste utilisable après réparation.
- **Lecture stdin cassée.** `request_read_failed` (code 74) et `request_too_large` (code 65) écrivent un enregistrement durable `host_aborted` lié à l’exécution, puis l’hôte sort au lieu de boucler.
- **Refus de budget durable.** Les appels normaux et le refus de budget passent par le même chemin de reçu. Un reçu absent ou une écriture échouée renvoie `receipt_write_failed` et arrête l’hôte.
- **Reçus atomiques par ligne.** Le trait `ReceiptSink` sérialise chaque enregistrement dans un tampon unique, puis fait un seul `write_all` suivi de `sync_data`. Le fichier est ouvert avec `append` + `create_new` + `O_NOFOLLOW` en `0600`, et le répertoire parent est synchronisé avant le premier enregistrement.
- **État hors du sandbox.** `refuse_state_paths_inside_workspace` refuse, avant le lease, un `journal_file` ou un `receipt_file` qui se résout (parent canonique + nom, plus le chemin complet s’il existe) dans `base`, `candidate`, `target` ou `rust_sysroot`. Les `..` et les symlinks sont résolus.

## Tests de régression

Nouveaux tests d’intégration dans `tests/mission_012_mcp_lifecycle.rs` :

| Test | Comportement vérifié |
| --- | --- |
| `mcp_host_refuses_journal_or_receipt_paths_inside_the_workspace` | Reçu dans `candidate`, reçu via un alias symlink vers `target`, journal via `candidate/../target`, journal symlink vers la base. Code 65, aucun lease, aucun fichier créé, base inchangée. |
| `mcp_host_refuses_an_unready_candidate_without_burning_the_lease` | Refus sans lease, puis succès d’acquisition avec le même ID après réparation. |
| `mcp_host_exits_and_records_an_abort_when_stdin_cannot_be_read` | stdin est un répertoire (EISDIR) ; code 74, une seule réponse, reçus `[execution_started, host_aborted]`. Une échéance de 30 s tue l’hôte en cas de boucle. |
| `mcp_host_exits_and_records_an_abort_on_an_oversized_frame` | 128 KiB + 1 octet sans saut de ligne ; code 65, reçus `[execution_started, host_aborted]`. |
| `mcp_host_persists_a_tool_budget_refusal_and_stops` | 12 appels réussis, le 13e est refusé et persisté ; arrêt sans réponse à la requête suivante ni finalisation. |

Nouveaux tests unitaires dans le binaire :

| Test | Comportement vérifié |
| --- | --- |
| `receipt_write_failures_stop_the_host_for_calls_and_budget_refusals` | Sink en échec, sur le chemin normal et sur le chemin de budget. |
| `finalization_reports_failure_when_its_receipt_cannot_be_written` | La finalisation échoue quand son reçu ne peut pas être écrit. |

Le test de budget existant vérifie désormais l’enregistrement persisté. L’assertion qui ne pouvait jamais échouer sur `workspace.diff` est remplacée.

## Limites explicites

L’en-tête du module les liste :

- **Même UID.** L’isolation n’est pas défendue : le lease est une barrière coopérative, et un processus du même UID peut écrire le workspace, le journal ou les reçus, ou échanger un répertoire après validation.
- **Opération en cours.** Une opération déjà en cours à l’expiration du lease n’est pas annulée ; son résultat est rejeté.
- **Finalisation occupée.** Elle échoue fermée, sans retry : `refused`, code 2.
- **Reçu créé après le claim.** Le fichier de reçus est créé après le lease. L’hôte ne relâche jamais son lease : toute sortie après acquisition le laisse tenu jusqu’au TTL, et une relance exige un nouvel ID.
- **Reçu après l’effet.** Le reçu suit l’effet : un kill entre les deux laisse un changement sans reçu, puis une relance refuse avec `candidate_not_at_base`.

Non traités et consignés dans la revue :

- un bind mount d’un répertoire du sandbox (root requis) ;
- une trame tronquée en EOF, traitée comme un EOF normal ;
- la troncature d’une queue non JSON par `read_locked` sur un journal désigné par la configuration (bibliothèque, hors périmètre).

## Clippy

Le contrôle indépendant de la racine (`evidence/webhook-mcp-claim-004-clippy.log`, conservé) avait signalé deux lints nouveaux avec la commande historique :

- `clippy::type_complexity` à `tests/mission_012_mcp_lifecycle.rs:707` : corrigé par l’alias de type `PlaceStatePath`.
- `clippy::nonminimal_bool` à `src/bin/mission-012-mcp-host.rs:928` : `!record.is_some_and(… .is_ok())` devient `record.is_none_or(… .is_err())`.

Aucune allowance n’a été ajoutée ; seules les trois allowances historiques restent passées en ligne de commande.

## Vérifications reproductibles

Versions observées : Rust/Cargo 1.95.0, bubblewrap 0.9.0. Dépendances épinglées par `Cargo.lock`.

| Commande | Résultat |
| --- | --- |
| `cargo test --offline --locked --test mission_012_mcp_lifecycle` | 13 réussis, 0 échec (8 tests précédents + 5 nouveaux). |
| `cargo test --offline --locked --bin mission-012-mcp-host` | 15 réussis, 0 échec. |
| `cargo test --offline --locked --all-targets` | 29 exécutables de test, 282 réussis, 0 échec, 9 ignorés. |
| `cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` | Passe, aucun avertissement. |
| `cargo build --offline --locked --release --all-targets` | Passe. |
| `cargo fmt --all -- --check` | Passe. |
| `git diff --check` + `git diff --no-index --check /dev/null <fichier>` sur les deux fichiers non suivis | Passe (journal vide). |

Sorties finales : `evidence/webhook-mcp-claim-004-{focused,bin-unit,all-targets,clippy,release-build,rustfmt,diff-check}-final.log`. Les empreintes sont dans `evidence/webhook-mcp-claim-004-sha256.txt`. Les journaux non suffixés `-final` sont des passages intermédiaires ; `webhook-mcp-claim-004-clippy.log` conserve l’échec initial des deux lints.

## Revue indépendante

Deux revues read-only Sonnet 5.5, avant et après correctifs : **0 finding HIGH/MEDIUM** dans chacune. Détails dans `evidence/webhook-mcp-claim-004-review.md`.

## Couverture

Tests processus locaux avec des fixtures. Aucun Forgejo live, appel modèle/LLM, PR, merge, commit ni secret n’a été utilisé.

## Re-vérification indépendante et limite de composition

À 2026-10-23 02:09 UTC, sur le même HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, la racine a relancé rustfmt, les deux suites ciblées, all-targets, Clippy, le build release et les contrôles d’espaces. Résultats frais : 13 tests d’intégration, 15 tests unitaires du binaire, 29 exécutables all-targets (282 réussis, 0 échec, 9 ignorés), Clippy sans avertissement avec les trois allowances historiques, build release et rustfmt réussis. Les journaux indépendants sont `evidence/webhook-mcp-claim-004-root-{rustfmt,focused,bin-unit,all-targets,clippy,release-build,diff-check}.log`. Environnement : rustc 1.95.0, Cargo 1.95.0, Bubblewrap 0.9.0. Les sources du lot n’ont pas changé entre les deux séries.

La revue distincte de composition a relevé un écart matériel : `WEBHOOK-CLI-ADMISSION-001` vise l’origine `127.0.0.1:3130`, `forge-admin/agent-forge-core`, issue 41 et `REQ-001`; `mission-012-mcp-host` épingle `127.0.0.1:3132`, `forge-admin/mission-012-clamp`, issue 1 et `MISSION-012-CLAMP-001`. `tests/mission_012_mcp_lifecycle.rs` fabrique son propre claim; aucun événement webhook ni résultat du CLI d’admission n’est consommé. Ce host durci ne complète donc pas le parcours WEBHOOK→MCP.

Autres limites vérifiées statiquement : le SHA de base de la requête est épinglé, mais le host ne prouve pas que `base_dir` représente ce commit (`CandidateToolHost::new` vérifie l’égalité base/candidat au démarrage; `verify_pair` n’est pas appelé par ce host); l’effet d’un outil précède l’écriture durable de son reçu, avec une fenêtre d’arrêt sans reçu; le journal/reçu n’est pas authentifié contre une modification par le même UID. Le prochain lot doit composer les bindings via une seule autorité hôte et lier le snapshot Git du workspace au commit avant l’exposition du contexte. Aucune conclusion de sécurité ou de succès général n’est déduite d’un seul scénario fixture.
