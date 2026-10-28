# CONTINUATION-030 — contrats webhook exposés par le CLI réconciliateur

Date : 26 octobre 2026. Checkout local : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Worktree préexistant fortement modifié, préservé.

## Changement

Avant ce lot, `forgejo-reconcile-local --capabilities` annonçait les noms pris en charge, mais n’exposait pas les contraintes nécessaires pour interpréter ces événements. Le refus `unsupported_event` disait seulement « issue, pull request, ou branch push » et ne dirigeait pas vers la découverte des capacités.

L’opération `reconcile_webhook_deliveries` publie maintenant, en plus du champ conservé `supported_events`, `event_contracts: {schema_version: 1, contracts: ...}` à partir de `SUPPORTED_EVENT_CONTRACTS`. L’erreur non réessayable expose le pointeur `operations[reconcile_webhook_deliveries].event_contracts` et invite à consulter `--capabilities`, sans suggérer de modifier le journal.

La propriété est additive au document `forgejo-reconcile-capabilities-v2`; cette tranche ne change pas son numéro de version et n’établit pas la compatibilité avec des clients externes. Le receiver et le CLI sérialisent les mêmes constantes compilées.

## TDD et vérification

Deux attentes ont été ajoutées avant l’implémentation : le test unitaire de formatage d’erreur et le test du processus réel `actual_cli_discloses_versioned_local_test_capabilities`. Les deux échouaient : action sans pointeur `--capabilities`, et `event_contracts.schema_version` absent (`capability-red.log`). Ils passent après ajout du champ et du pointeur. Le test processus compare toute la liste de contrats sérialisée à `SUPPORTED_EVENT_CONTRACTS`, tout en conservant l’assertion historique sur les neuf noms.

- `cargo test --locked --offline --bin forgejo-reconcile-local tests::unsupported_event_error_points_to_cli_capability_contract -- --exact --nocapture` — rouge avant, vert 1/1 après.
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --exact --nocapture` — rouge avant, vert 1/1 après.
- `cargo test --locked --offline --all-targets -- --test-threads=1` — 554 réussis, 0 échec, 76 ignorés, 34 groupes.
- `cargo fmt --all -- --check`, `git diff --check`, `python3 scripts/verify_cargo_license_state.py` — réussis.
- `cargo metadata --format-version 1 --locked --offline --no-deps` — `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`.

Toolchain : `rustc 1.95.0 (59807616e 2026-04-14)` et `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`. Les logs rouges/verts, suite, versions, readback et hashes sont conservés ici.

## Portée et limites

Le test exécute le binaire CLI local, sans réseau ni lecture d’état Forgejo. Les contrats sont les assertions compilées du produit; ils ne démontrent pas qu’une instance Forgejo distante respecte les formes annoncées. Aucun agent LLM/AgentOS, aucun coût nouveau, service externe ou benchmark comparatif n’intervient. Le projet reste incomplet.
