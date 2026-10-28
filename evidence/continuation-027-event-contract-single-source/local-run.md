# CONTINUATION-027 — allowlist webhook dérivée des contrats

Date d’exécution : 26 octobre 2026. Dépôt local : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; le lot a préservé cet état et n’a ni réinitialisé ni commité les changements.

## Changement

Avant ce lot, `src/webhook.rs` maintenait deux tableaux de neuf noms : `SUPPORTED_EVENTS` pour l’ingestion/réconciliation et `SUPPORTED_EVENT_CONTRACTS` pour la découverte machine-readable ajoutée en CONTINUATION-026. Les listes pouvaient diverger si un événement était ajouté à une seule d’entre elles.

`SUPPORTED_EVENT_CONTRACTS` est maintenant la source unique des noms. La constante publique `SUPPORTED_EVENTS` est calculée à la compilation en parcourant ces contrats; la taille de son tableau est dérivée de `SUPPORTED_EVENT_CONTRACTS.len()`. Les consommateurs, l’API et le contenu des capacités restent identiques. Aucun flux HTTP, événement admis, payload ni verdict n’a été changé.

Il s’agit d’un refactor à comportement préservé : aucun test fonctionnel rouge distinct n’est pertinent, et aucun nouveau test n’a été ajouté. Le test existant `capabilities_publish_machine_readable_contracts_for_every_supported_event` compare l’ordre et l’égalité des événements annoncés et contractuels; il passe après le changement.

## Vérifications reproductibles

- Test ciblé : `cargo test --locked --offline --test webhook_handler_contract capabilities_publish_machine_readable_contracts_for_every_supported_event -- --exact --nocapture` — 1 réussi, 0 échec.
- Suite : `cargo test --locked --offline --all-targets -- --test-threads=1` — 553 réussis, 0 échec, 76 ignorés, 34 groupes.
- `cargo fmt --all -- --check` — réussi.
- `git diff --check` — réussi.
- `python3 scripts/verify_cargo_license_state.py` — réussi; état `no_license_selected`, droits de redistribution `not_assessed`.
- `cargo metadata --format-version 1 --locked --offline --no-deps` — `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`.

Outils : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`. Les sorties brutes, le readback Cargo et les commandes sont conservés dans ce dossier.

## Portée et limites

Le test vérifie l’égalité interne des deux vues de capacités et la suite vérifie les consommateurs locaux; il ne prouve pas que chaque version de Forgejo émet réellement les neuf événements annoncés. Il ne vérifie ni Forgejo live, ni AgentOS, ni un parcours avec LLM, ni une comparaison contrôlée ou un gain de productivité. Les 76 tests ignorés ne sont pas exécutés par la commande all-targets. Le projet demeure incomplet.
