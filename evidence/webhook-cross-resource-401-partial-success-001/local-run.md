# WEBHOOK-CROSS-RESOURCE-401-PARTIAL-SUCCESS-001 — résultat local

Date : 25 octobre 2026. Le test processus ajouté traverse le receiver webhook réel, son journal durable et le CLI Rust `forgejo-reconcile-local`. L’API Forgejo reste une fixture HTTP loopback synthétique. Le binaire est compilé depuis le worktree puis exécuté sous Bubblewrap 0.9.0 avec namespaces réseau et PID séparés; aucune requête externe n’est autorisée.

## Comportement vérifié

Le lot contient une issue #41, une PR #41, une issue #42, puis un doublon de l’issue #41 et un doublon de la PR #41. L’API fixture renvoie l’état courant `closed` pour l’issue #41, puis HTTP 401 pour la PR #41. Le CLI émet ensuite : l’état canonique de l’issue #41, `upstream_unauthorized` pour la PR #41 et l’issue #42 (aucune troisième requête après le latch 401), de nouveau l’état `closed` depuis le cache pour le doublon de l’issue, et le refus mis en cache pour le doublon de la PR. Le verdict global du lot est `inconclusive`; seuls deux GET authentifiés ont été tentés. Les cinq IDs restent dans l’ordre et l’inbox est identique octet pour octet après le run.

Le marqueur synthétique du corps HTTP 401 et le token pipe-only n’apparaissent ni dans stdout ni stderr. Le test ciblé passe : 1 réussi, 0 échec, 0 ignoré, 47 filtrés. Cette preuve formalise la conservation d’une lecture réussie en présence d’un refus d’authentification d’une ressource sœur et la distinction des ressources issue/PR ayant le même numéro.

Commande du runner :

```sh
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_preserves_successful_issue_when_pull_read_latches_401
```

Vérifications de la tranche : `cargo fmt --all -- --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, `git diff --check`, garde Cargo et sept tests de garde. Le readback Cargo dit `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`; aucun choix de licence n’est fait.

## Portée et limites

Cette tranche modifie seulement le test processus et sa liste de scénarios. Elle ne change pas le code d’exécution du réconciliateur; elle prouve un contrat déjà implémenté par le cache par ressource et le latch de 401. L’instance API est une fixture, pas Forgejo réel. Les payloads, token et corps de refus sont synthétiques. Le test n’établit ni un parcours LLM/AgentOS, ni un bénéfice de productivité, ni un benchmark comparatif.

## Fichiers de preuve

- `runner.log` : compilation isolée, commande Bubblewrap et résultat du test.
- `cargo-license-readback.json`, `cargo-license-tests.log`, `tool-versions.txt` : lecture de la décision de licence et versions courantes.
- `commands.txt` et `source-sha256.txt` : commande de reproduction et empreintes des sources.
- `SHA256SUMS` : manifeste local.
