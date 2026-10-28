# WEBHOOK-PR-STATE-CONSISTENCY-001 — refuser un PR ouvert déclaré fusionné

## Objet

La réconciliation Forgejo produisait un état canonique quand sa réponse donnait simultanément `state="open"` et `merged=true`. Ces champs ne sont pas cohérents pour une pull request fusionnée. L’adaptateur doit échouer fermé et ne pas émettre de `canonical_state`.

## Changements

- `src/webhook/reconcile.rs` renvoie `InvalidCurrentState` lorsque `merged=true` et que l’état n’est pas `closed`. Les PR fermées sans fusion restent valides.
- `tests/webhook_reconcile_contract.rs::open_pull_request_cannot_be_reported_as_merged` vérifie l’invariant.
- `tests/webhook_receiver_process_e2e.rs::actual_reconcile_cli_process_refuses_open_pull_request_reported_as_merged` vérifie la réponse du CLI après réception/journalisation d’un webhook et lecture de l’API fixture.
- `scripts/run_webhook_reconcile_cli_e2e.py` expose le cas par son nom dans `--scenario`.

Le test de contrat a d’abord échoué avant le changement : l’exécution renvoyait `Ok(ReconciledState { state: "open", merged: Some(true), ... })` au lieu de `Err(InvalidCurrentState)`. Après le correctif, le contrat entier passe 23/23, dont un cas positif qui préserve `state="open", merged=false`.

## Vérifications

| Commande | Résultat |
|---|---|
| `cargo test --offline --locked --test webhook_reconcile_contract` | 23 réussis, 0 échec |
| `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_open_pull_request_reported_as_merged` | 1 réussi sous Bubblewrap 0.9.0, `--unshare-net`; journal [process-e2e.log](process-e2e.log) |
| `cargo test --offline --locked --all-targets` | 438 réussis, 0 échec, 30 ignorés dans 34 groupes; journal [all-targets.log](all-targets.log) |
| `cargo fmt --all -- --check` | passe |
| `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` | passe |
| `git diff --check` | passe |
| `python3 scripts/verify_cargo_license_state.py` | `license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits `not_assessed` |

Environnement : HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree modifié; `rustc 1.95.0 (59807616e 2026-04-14)`, Cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0, Linux aarch64. Les sources locales exactes sont liées par ces SHA-256 :

```text
8d51c40dfce84a25d29453761b5a60911034e6cd45b577740b3ccfa4c1bc1c1d  src/webhook/reconcile.rs
f0738f4d24b51078aef70f13d0dc5dab4dadc1cf72fd8ee9f58b23d8a066253a  tests/webhook_reconcile_contract.rs
439500b3bb22f66f9a8193dd4e38731dbbd038b3d8ac7b259bfc4eb8a9959c5d  tests/webhook_receiver_process_e2e.rs
cb42e8e18dc613551759a4d4160a3b2947a9fd34924a4475d9da12157856a781  scripts/run_webhook_reconcile_cli_e2e.py
```

## Portée et limites

Le processus receiver et le CLI sont réels, mais l’API Forgejo est une fixture HTTP loopback; le sandbox n’a pas de réseau externe et aucune credential réelle n’est utilisée. Ce scénario ne lance pas Forgejo, ne crée pas de PR et ne prouve pas le parcours agentique complet. Aucun LLM, AgentOS, RAG ou mémoire n’est utilisé. Il n’établit aucun gain comparatif, temps, token ou coût. Les sources modifiées restent dans le worktree non commité; aucune licence Cargo n’a été sélectionnée.
