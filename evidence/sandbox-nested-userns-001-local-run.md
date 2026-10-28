# SANDBOX-NESTED-USERNS-001 — refus des namespaces utilisateur imbriqués

Date : 24 octobre 2026. Base Git observée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà modifié et contenait de nombreux fichiers non suivis; aucun nettoyage, commit, push ni changement Forgejo n’a été fait.

## Constat et changement

L’exécution d’acceptance de `CandidateToolHost` utilisait Bubblewrap avec `--unshare-all`, les mounts limités, le home absent, le réseau isolé, la source en lecture seule et des limites par processus. Sur cet hôte, `bwrap 0.9.0` laissait néanmoins le processus lancer un autre `unshare --user --map-root-user /usr/bin/true` dans ce bac.

[`run_sandbox_test`](../src/mission_012_host.rs#L1208) passe maintenant explicitement `--unshare-user --disable-userns --assert-userns-disabled`. Le premier exige le namespace utilisateur extérieur et le second configure la désactivation des namespaces imbriqués; le troisième demande à Bubblewrap de refuser le lancement si cette propriété ne peut pas être confirmée. La version locale installée est Bubblewrap `0.9.0`; ses options documentées déclarent cette dépendance et cet effet ([manuel v0.9.0](https://github.com/containers/bubblewrap/blob/v0.9.0/bwrap.xml)). Les mainteneurs décrivent Bubblewrap comme un outil bas niveau dont le niveau de sécurité dépend de la configuration de l’appelant, pas comme un sandbox complet prêt à l’emploi ([politique de sécurité v0.9.0](https://github.com/containers/bubblewrap/blob/v0.9.0/SECURITY.md)).

Le nouveau test [`sandbox_blocks_nested_user_namespace_creation`](../src/mission_012_host.rs#L2365) substitue uniquement un faux exécutable Cargo dans le sysroot de test. Celui-ci tente réellement de créer un namespace imbriqué depuis le sandbox et inscrit `allowed` ou `blocked` dans `/target`. Le test a d’abord échoué avec `allowed`, puis passe avec `blocked`. Les deux anciens tests de sandbox rouge/vert et de refus d’un target chevauchant passent également. La commande de production, le sysroot et l’entrée restent fixes dans l’hôte; le modèle ne choisit ni shell ni commande.

## Vérifications

- Rouge : `cargo test --locked --offline --bin mission-012-mcp-host host::tests::sandbox_blocks_nested_user_namespace_creation -- --exact --nocapture` — 1 test échoue comme attendu, car le probe renvoie `allowed` ([log](sandbox-nested-userns-001-red.log)).
- Vert ciblé : `cargo test --locked --offline --bin mission-012-mcp-host host::tests::sandbox_ -- --nocapture --test-threads=3` — 3 réussis ([log](sandbox-nested-userns-001-focused.log)).
- Host MCP en processus : `cargo test --locked --offline --test mission_012_mcp_lifecycle -- --test-threads=4` — 24 réussis, dont le cycle MCP rouge → écriture → vert ([log](sandbox-nested-userns-001-mcp-process.log)).
- Suite finale : `cargo test --locked --offline --all-targets -- --test-threads=4` — 33 groupes, 411 réussis, 0 échec, 18 ignorés ([log](sandbox-nested-userns-001-all-targets.log)). Les E2E ignorés ont leurs runners privés distincts; ils ne sont pas comptés comme exécutés par cette commande.
- `cargo clippy --locked --offline --all-targets -- -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` et `cargo build --locked --offline --release --all-targets` — code 0 ([Clippy](sandbox-nested-userns-001-clippy.log), [build release](sandbox-nested-userns-001-release.log)). Clippy conserve deux warnings déjà présents hors du changement : `collapsible_if` dans `src/agent_runtime.rs:1335` et `enum_variant_names` dans `tests/forgejo_merge_post_crash_recovery_process_e2e.rs:300`.
- `rustfmt --edition 2024 --check src/mission_012_host.rs` et `git diff --check` passent sur l’état final. `cargo metadata --locked --offline --no-deps` conserve `license=null`, `license_file=null`, `publish=[]`.

## Portée

Cette modification interdit une capacité de namespace imbriqué et réduit ainsi une partie de la surface d’attaque du noyau. Elle ne constitue pas un audit ou une certification de Bubblewrap et ne garantit pas la sécurité contre une vulnérabilité du noyau, de Bubblewrap ou du compilateur. Elle n’isole pas l’agent du processus hôte partageant le même UID; le target persistant n’a pas de quota disque total et `RLIMIT_AS` borne l’espace d’adressage de chaque processus, sans plafond de mémoire résidente agrégé par mission. Le run passe par un faux Cargo pour sonder la propriété et par le test d’acceptance local existant; il n’utilise pas Forgejo, un LLM, AgentOS ou des données externes.

La session Claude/Ralph `1947069` n’a pas été touchée. Le dernier contrôle passif a trouvé le PID présent (`Ssl+`), CPU arrondi `00:03:26` contre `00:03:22`, marqueur inchangé à l’itération 1/6. Ces indicateurs ne prouvent pas un progrès utile ou un blocage. Aucun nouveau modèle n’a été invoqué pour ce lot.
