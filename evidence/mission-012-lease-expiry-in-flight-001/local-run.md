# MISSION-012-LEASE-EXPIRY-IN-FLIGHT-001

Date : 25 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt reste fortement modifié et les sources de ce prototype ne sont pas suivies par Git. Aucun commit, publication, déploiement, appel de modèle ou accès externe n’a eu lieu.

## Comportement vérifié

Le test processus démarre `mission-012-mcp-host` avec son admission et ses snapshots liés au dépôt Git de fixture. Il envoie une requête MCP `cargo.test` pendant un bail de 3 secondes. Dès que le processus `systemd-run` du vérificateur est présent, le test suspend seulement l’hôte; le vérificateur utilise son groupe de processus distinct et termine. Après expiration, le test reprend l’hôte et ferme l’entrée MCP.

La réponse est `execution_lease_expired` avec `details.during_operation=true`. Le journal durable contient une résolution de réservation CPU, donc l’invocation interne a rendu son résultat; le reçu `cargo.test` associé au même appel est cependant `success=false`, `error_code=execution_lease_expired`, avec une durée d’au moins 3 000 ms. La finalisation est `refused` pour le même code. La suite vérifie également que les tests et snapshots admis n’ont pas été remplacés pour ralentir artificiellement l’acceptance.

Le test prouve le rejet d’un résultat revenu après expiration. Il ne prouve pas l’annulation du vérificateur : dans ce scénario, le sous-processus continue jusqu’à sa fin. La limite connue du host — un outil déjà lancé n’est pas tué automatiquement à l’expiration du lease — demeure.

## Vérifications

- `cargo test --offline --locked --test mission_012_mcp_lifecycle mcp_host_discards_a_successful_tool_result_when_lease_expires_in_flight -- --exact --nocapture` : 1 réussi, 27 filtrés; voir `focused-final.log`.
- `cargo test --offline --locked --test mission_012_mcp_lifecycle -- --test-threads=1` : 27 réussis, 0 échec, 1 ignoré; 15,64 s; voir `lifecycle-suite-final.log`.
- `cargo clippy --offline --locked --test mission_012_mcp_lifecycle -- -A clippy::too_many_arguments -A clippy::collapsible_if -A clippy::bool_assert_comparison -A clippy::manual_div_ceil -A clippy::enum_variant_names -A clippy::type_complexity` : réussite; les six allowances sont celles documentées dans les lots précédents.
- `cargo fmt --all -- --check` : réussite.
- `rustfmt --edition 2024 tests/mission_012_mcp_lifecycle.rs` : appliqué au seul fichier de test modifié.

Les journaux `focused.log` et `lifecycle-suite.log` conservent également les premiers runs réussis, avant que les assertions ne vérifient explicitement `during_operation=true` et la résolution CPU. Les runs finaux sont ceux qui correspondent à la source finale. Les versions de Rust, Cargo, Git et du noyau sont dans `toolchain.txt`; les empreintes source finales figurent dans `source-sha256.txt`. `SHA256SUMS` couvre les éléments de preuve locaux sauf lui-même.

## Limites et suite

Ce test est un seul scénario Linux, dans un dépôt et des processus de fixture locaux. Il n’exerce pas un signal d’annulation client, SIGTERM de l’opérateur, l’arrêt/nettoyage du groupe du vérificateur à la mort de l’hôte, un processus malveillant du même UID, AgentOS ou un LLM, une PR ou une fusion Forgejo. Il n’apporte aucune mesure comparative Forgejo + agent + CI et ne démontre aucun gain global.

Audit statique complémentaire, à distinguer du test observé : `src/mission_012_host.rs::run_sandbox_test` applique `.process_group(0)` au processus `systemd-run` (ligne 1538) et `spawn_capture` tue ce groupe en cas de délai (lignes 1013–1017). `scripts/mission_012_mcp_supervisor.py::terminate_host_group` envoie SIGTERM puis SIGKILL au groupe de l’hôte (lignes 31–53); le superviseur crée l’hôte avec `start_new_session=True` (ligne 85). Cela laisse ouverte l’hypothèse qu’un arrêt forcé de l’hôte ne cible pas le groupe du vérificateur. Le cycle exact du scope systemd et de ses descendants n’a pas été exécuté après mort de l’hôte; aucune fuite n’est donc revendiquée. Prochaine vérification : annuler le superviseur pendant `cargo.test`, observer le groupe vérificateur, puis prouver que tous ses processus finissent ou corriger le nettoyage dans le périmètre autorisé.
