# WEBHOOK-REPOSITORY-IDENTITY-PROCESS-001 — réception réelle avant écriture durable

Date : 25 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre préexistant modifié. Aucun fichier n’a été stagé ou commité.

## Contrat exercé

Ce lot prolonge le correctif de cohérence `repository.full_name` / `repository.name` / `repository.owner.login` déjà vérifié dans [FORGEJO-WEBHOOK-REPOSITORY-IDENTITY-001](../forgejo-webhook-repository-identity-001/local-run.md). Le test démarre le véritable binaire local `forgejo-webhook-local` avec un secret HMAC de fixture, un journal privé neuf et un listener loopback. Une forme cohérente reçoit HTTP 202 et entre dans le journal. Deux corps signés valides mais contradictoires — nom de dépôt puis login du propriétaire — reçoivent chacun HTTP 400 `event_payload_invalid`, `retryable=false`. Les octets du journal restent identiques après chaque refus. Après arrêt du processus, le replay contient uniquement la livraison valide.

Le JSON est une fixture synthétique construite à partir des champs wire audités dans Forgejo v17.0.0 (`modules/structs/repo.go::Repository`, `modules/structs/user.go::User`, `services/convert/repository.go::ToRepo`, source au commit `0d791421f85ca13a63b665053d81ea765fbbc571`). Aucun serveur Forgejo n’a émis ce corps pendant ce test; il s’agit d’une preuve du chemin processus receiver, pas d’un E2E Forgejo natif.

## Commandes et résultats

- `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_binds_repository_identity_fields_before_durable_append -- --exact --test-threads=1`: 1 réussi, 0 échec.
- `cargo test --offline --locked --test webhook_receiver_process_e2e -- --test-threads=1`: 17 réussis, 0 échec, 50 ignorés. Les cas ignorés exigent `bwrap --unshare-net` et réservent un endpoint d’API fixe; ils n’ont pas été exécutés.
- `cargo test --offline --locked --test webhook_contract --test webhook_reconcile_contract --test webhook_receiver_process_e2e -- --test-threads=1`: 9 + 35 + 17 réussis, 0 échec; 50 ignorés dans le binaire de processus.
- `rustfmt --edition 2024 --check tests/webhook_receiver_process_e2e.rs`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py`: succès.
- Rust/Cargo `1.95.0`; preuves de commandes et sorties : `commands.txt`, `toolchain.txt`, `targeted-test.log`, `process-suite.log`, `combined-contracts.log`.

Le garde de licence lit toujours `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`; aucune licence n’a été attribuée. Cargo a tourné en mode offline/verrouillé. Aucun LLM, AgentOS, RAG, mémoire, service externe, instance de production, credential privé ou coût nouveau n’a été utilisé.

## Portée et limites

Cette preuve ferme le passage handler → vrai processus → journal durable pour les deux contradictions ciblées, avec une seule instance locale, sous le même UID, et trois livraisons de fixture. Elle ne prouve pas le contenu de tous les événements natifs Forgejo, l’isolation contre un processus hostile du même UID, la concurrence multi-hôte ou une intégration AgentOS. Les 50 tests ignorés n’ont pas été rejoués dans ce lot; la suite Cargo complète n’a pas été exécutée. La prochaine tranche doit viser une lacune distincte du contrat d’adaptateur sans rejouer ces scénarios d’identité.
