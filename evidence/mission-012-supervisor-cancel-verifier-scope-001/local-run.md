# MISSION-012-SUPERVISOR-CANCEL-VERIFIER-SCOPE-001 — annulation durant une vérification en vol

Date : 25 octobre 2026. Commit observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; worktree fortement modifié. Le test ajouté est `tests/mission_012_mcp_lifecycle.rs::mcp_supervisor_cancellation_terminates_the_in_flight_verifier_scope`.

## Expérience et constat

Le test démarre le vrai `scripts/mission_012_mcp_supervisor.py`, le binaire `mission-012-mcp-host` et une requête MCP `cargo_test`. Il trouve sous le host le scope transitoire `run-*.scope` créé par `systemd-run`, énumère les PIDs du cgroup, puis les suspend tous par SIGSTOP pour garantir que le vérificateur reste en vol. Il envoie SIGTERM au superviseur et attend le délai réel de fermeture bornée. Le superviseur sort avec le code 75. Avant que le garde de test `VerifierCgroupCleanup` soit détruit, l’énumération `/proc` du même cgroup ne retourne aucun PID.

La fuite envisagée d’un vérificateur systemd après terminaison du host n’est donc pas reproduite avec le superviseur, la session systemd et le noyau/cgroup observés. Le test n’infère pas quel composant a fait disparaître le scope et ne prouve pas la terminaison de descendants qui changeraient de cgroup, d’un processus arbitraire du même UID ou d’un autre système d’init.

## Vérifications

`cargo test --offline --locked --test mission_012_mcp_lifecycle` : 28 passés, 0 échec, 1 ignoré (cas réservé au harness d’isolation). Le test focalisé passe aussi 1/1. `rustfmt --edition 2024 --check` et whitespace passent.

Clippy strict `-D warnings` échoue sur dix diagnostics qui précèdent ce changement et sont localisés hors du code ajouté : `too_many_arguments` dans `src/agent_runtime.rs` (3), `src/forgejo.rs` (1), `src/mission_admission.rs` (3), `src/lib.rs` (1), puis `collapsible_if` dans `src/agent_runtime.rs` (2). Avec seulement ces deux lint classes baseline autorisées, Clippy ciblé passe (`clippy-with-baseline-allowances.log`). Le changement est limité au test processus; aucun code de production ou superviseur n’a été modifié.

Le runtime utilise Rust/Cargo 1.95.0, systemd-run et cgroup v2; détails de version, commande et empreintes sont fournis avec les logs. Aucun modèle, service externe ou coût nouveau n’est engagé. L’état de licence Cargo demeure sans choix (`license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`) et n’a pas été modifié.

## Portée et suite

Cette vérification exerce une annulation SIGTERM réelle avec le vérificateur bloqué dans son scope systemd et observe qu’aucun processus ne reste dans ce cgroup à la sortie 75. Elle ne couvre pas l’annulation client JSON-RPC distincte, tous les états intermédiaires du journal, d’autres gestionnaires de service, des descendants qui s’échappent du cgroup, ni une frontière multi-tenant. Aucun gain produit ni parcours LLM n’est démontré.

Les vérifications locales restantes des frontières de concurrence/capacité et le protocole comparatif conventionnel restent à poursuivre; ce run seul n’achève pas la mission.
