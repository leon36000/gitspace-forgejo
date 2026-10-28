# WEBHOOK-FIFO-PROCESS-001 — refus non bloquant d’une entrée FIFO

Date : 24 octobre 2026. HEAD examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree préexistant très modifié. Aucun commit, push, déploiement, accès Forgejo ni appel de modèle.

## Résultat

Le parseur du CLI de préflight exige des fichiers d’entrée ordinaires. `read_adapter_input` ouvre déjà avec `O_NONBLOCK | O_NOFOLLOW`, puis vérifie le type avec `fstat` avant la lecture. Le test de processus absent des tests canoniques lance le vrai binaire `forgejo-reconcile-local` avec un FIFO comme fichier `expected.json`; il impose une limite de deux secondes et vérifie un refus actionnable signalant l’exigence de fichier ordinaire. Le FIFO temporaire est créé dans un répertoire propre au test et retiré à sa fin.

Vérifications fraîches, en Rust/Cargo `1.95.0`, hors ligne et verrouillées :

- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e preflight_rejects_fifo_input_without_blocking -- --exact --nocapture` : 1 réussi en 0,01 s; [journal](webhook-fifo-process-e2e.log).
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e` : 3 réussis, 0 échec; [journal](webhook-capabilities-contract.log).
- `cargo test --locked --offline --test webhook_reconcile_contract` : 20 réussis, 0 échec; [journal](webhook-reconcile-contract.log).
- `cargo test --locked --offline --all-targets -- --test-threads=4` : 33 groupes, 405 réussis, 0 échec, 17 ignorés; [journal](webhook-fifo-process-all-targets.log). Les tests processus explicitement ignorés sont réservés à leurs runners Bubblewrap.
- `rustfmt --check --edition 2024 tests/forgejo_reconcile_capabilities_e2e.rs` et `git diff --check` : réussis.
- `python3 scripts/verify_cargo_license_state.py` : `status=verified`, `license=null`, `license_file=null`, `publish=[]`, décision non choisie; [journal](webhook-fifo-cargo-license-guard.log).

Le code du lecteur existait déjà; ce lot ajoute seulement un test de processus au fichier d’intégration local. Le test FIFO s’exécute sur Unix. Il n’appelle pas l’API Forgejo, ne prouve pas le parcours complet, et ne change pas le modèle de sécurité du processus hôte. Il ne constitue ni une vérification globale du dépôt ni une mesure de valeur de la forge.
