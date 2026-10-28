# CONTINUATION-031 — enveloppe webhook commune au receiver et au CLI

Date : 26 octobre 2026. Checkout local : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree préexistant était fortement modifié et a été préservé.

## Écart et changement

Le receiver exposait `event_contracts` avec `schema_version=1`, `webhook_payload_is_authoritative=false` et les contrats. CONTINUATION-030 avait ajouté les contrats au CLI, mais son envelope omettait le champ de confiance : les deux points de découverte donnaient une description différente du webhook.

`src/webhook.rs` définit maintenant le type sérialisable `WebhookEventContractsV1` et la valeur canonique `WEBHOOK_EVENT_CONTRACTS_V1`. Le receiver et l’opération `reconcile_webhook_deliveries` sérialisent cette même valeur. Le document parent du CLI demeure `forgejo-reconcile-capabilities-v2`; les anciens champs sont conservés et le contrat d’événement garde son propre `schema_version=1`.

## Vérification

Le test processus CLI a d’abord échoué avec `null` au lieu de `false` pour `webhook_payload_is_authoritative` (`red.log`). Le test handler receiver et le test processus CLI comparent ensuite l’enveloppe aux constantes partagées et vérifient explicitement le marqueur non autoritatif. Le test processus du vrai receiver confirme le même envelope sans modifier l’inbox.

- `cargo test --locked --offline --test webhook_handler_contract capabilities_publish_machine_readable_contracts_for_every_supported_event -- --exact --nocapture` — 1 réussi.
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --exact --nocapture` — 1 réussi.
- `cargo test --locked --offline --test webhook_receiver_process_e2e capabilities_endpoint_describes_the_versioned_webhook_contract_without_mutating_inbox -- --exact --nocapture` — 1 réussi.
- `cargo test --locked --offline --all-targets -- --test-threads=1` — 554 réussis, 0 échec, 76 ignorés, 34 groupes.
- `cargo fmt --all -- --check`, `git diff --check`, `python3 scripts/verify_cargo_license_state.py` — réussis.
- `cargo metadata --format-version 1 --locked --offline --no-deps` — `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`.

Toolchain : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`. Les logs, le readback metadata, les versions et les empreintes sont archivés dans ce dossier.

## Limites

`webhook_payload_is_authoritative=false` exprime la règle du client; ce booléen ne prouve ni l’authenticité d’un événement ni la conformité d’un producteur distant. Aucun client externe ou instance Forgejo distante n’a été testé. Aucun changement de réconciliation, AgentOS/LLM, coût ou benchmark n’intervient. Cargo ne sélectionne aucune licence et les droits de redistribution restent `not_assessed`; le projet demeure incomplet.
