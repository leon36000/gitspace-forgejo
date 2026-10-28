# Vérifications Rust — worktree local courant

## Suite Cargo hors ligne

- Horodatage : 2026-10-24 13:54:53 UTC.
- Commande : `cargo test --locked --offline` — code de sortie `0`.
- Rust : `rustc 1.95.0 (59807616e 2026-04-14)`; Cargo : `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`.
- HEAD : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree déjà modifié lors de l’exécution.
- 35 résumés Cargo : 426 passés, 0 échec, 22 ignorés par la commande générique.
- Sortie brute : [cargo-test.log](cargo-test.log).

## Tests de processus isolés

- Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py` — code de sortie `0`.
- Bubblewrap : `/usr/bin/bwrap`, version `0.9.0`. Compilation Cargo hors ligne; les 12 tests ignorés ciblés s’exécutent chacun dans `bwrap --unshare-net`, racine en lecture seule et `/tmp` temporaire.
- Résultat : 12 scénarios, 12 passés, 0 échec. Le lot couvre notamment événements hors ordre, reprises sur journal inchangé après interruption, annulation de lectures, credentials interdits et retries bornés.
- Journal avec commandes exactes : [isolated-process-e2e.log](isolated-process-e2e.log).

Le runner a donc exécuté 12 des 22 tests marqués `ignored` par la commande Cargo générique; les 10 autres tests ignorés ne sont pas couverts par ces deux commandes. La suite générique et le runner ciblé ne démontrent pas un parcours LLM, une intégration AgentOS ni un gain comparatif. Les empreintes des manifestes, sources Rust et runner sont conservées dans [source-sha256.txt](source-sha256.txt).
