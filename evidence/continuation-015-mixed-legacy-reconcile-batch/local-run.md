# CONTINUATION-015 — poursuivre un lot malgré une entrée inbox historique invalide

Date du run : 2026-10-25 (EDT). HEAD du worktree : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; les sources de test et runner ne sont pas commités, leurs empreintes sont dans `source-sha256.txt`.

## Question et résultat

Une entrée invalide dans un lot sélectionné empêche-t-elle aussi la lecture d’un événement indépendant valide? Le test de processus `actual_reconcile_cli_process_continues_batch_after_legacy_issue_review_member` démarre le vrai receiver, accepte deux événements issue signés, puis modifie le second en ajoutant `review: null` et recalcule le hash de sa ligne. La chaîne du journal reste lisible. Le vrai CLI reçoit les deux identifiants dans l’ordre invalide puis valide.

Résultat : le CLI termine avec un verdict global `inconclusive`; le premier résultat est `invalid_envelope`, non réessayable, sans état canonique. Il continue toutefois et lit l’état actuel de l’issue valide par un seul GET, qui retourne `current_state_read`. Le fixture constate exactement un GET sur `/issues/41`, sans en-tête Authorization; le journal est identique avant et après le CLI. Le scénario passe `1/1`, avec 77 tests filtrés.

## Reproduction

Commande :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_continues_batch_after_legacy_issue_review_member
```

Le runner compile les binaires et test ciblés depuis une copie de sources en lecture seule avec Cargo `--offline --locked`, puis exécute le test sous Bubblewrap avec `--unshare-net --unshare-pid`, limites de processus/CPU/mémoire et uniquement le listener loopback de fixture dans cet espace réseau. Sortie complète : `runner.log`.

Contrôles complémentaires exécutés :

- `cargo fmt --all -- --check` — succès (`fmt-check.log`)
- `git diff --check` — succès (`diff-check.log`)
- `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` — succès (`py-compile.log`)
- `python3 scripts/test_cargo_license_state.py` — 9/9 (`cargo-license-tests.log`)
- `python3 scripts/verify_cargo_license_state.py` — `verified`; Cargo `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`, `NOASSERTION` (`cargo-license-verifier.json`)
- `cargo metadata --no-deps --format-version 1` — readback des champs Cargo dans `cargo-metadata-readback.json`

Environnement exact : `tool-versions.txt`. Empreintes SHA-256 du test, du runner et des sources métier observées : `source-sha256.txt`.

## Portée et réserves

Le lot ajoute un test d’intégration processus et son nom au runner allowlisté; aucune logique de production ne change. La ligne « historique » est synthétique : elle est générée avec le receiver courant, mutée par le test, puis re-hachée avec le mécanisme courant. Ce n’est pas un journal capturé d’une ancienne version, le format ne porte pas une signature HMAC historique et la chaîne de hash ne prouve pas l’identité de l’émetteur. Le test prouve la réponse du CLI à cette forme construite et la poursuite d’une lecture indépendante, pas la compatibilité de tous les anciens formats.

Le serveur HTTP testé est une fixture locale, pas Forgejo. Aucun modèle, AgentOS, coût ou credential réel n’intervient. Le scénario ne prouve pas de parcours LLM, comportement après fusion, gain comparatif, confinement hôte/agent, ni couverture complète des erreurs de lot. Les rapports précédents restent la preuve distincte des autres refus et du parcours composé; ce test ne les répète pas.
