# CONTINUATION-397 — batch mixte tag delete / branche delete

Date : 28 octobre 2026. Worktree local `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout était déjà sale; aucun autre changement n’a été réinitialisé, indexé ou commité.

## Question vérifiée

Une livraison tag `delete` provenant d’un ancien journal est refusée avant tout appel API. Dans un batch, ce refus par livraison doit laisser les livraisons de branche voisines continuer, quel que soit leur ordre.

Le test `batch_reconciliation_keeps_unsupported_tag_delete_local_and_continues_branch_delete` construit avec les fixtures de contrat :

- un `delete` tag (`refs/tags/v1.0.0`, `ref_type=tag`) et un `delete` de branche (`feature/agent-proof`, `ref_type=branch`);
- un `FixtureApi` mémoire qui renvoie le head courant `cccc…cccc`;
- deux exécutions du batch, tag d’abord puis branche d’abord.

Dans les deux ordres, le tag reste `UnsupportedEventPayloadField("ref_type")`. La branche retourne `present` au SHA `cccc…cccc`. Le fixture observe exactement deux requêtes au total, toutes deux vers la ressource de branche : aucune lecture API n’est faite pour le tag.

## Commandes et résultats

```text
cargo test --offline --locked --test webhook_reconcile_contract batch_reconciliation_keeps_unsupported_tag_delete_local_and_continues_branch_delete -- --exact --nocapture
cargo test --offline --locked --test webhook_reconcile_contract -- --test-threads=1
```

Le test exact passe 1/1 (`mixed-tag-branch-batch.log`, 98 autres tests filtrés). Après avoir corrigé le seul retour à la ligne signalé par le premier contrôle rustfmt, la cible entière passe encore 99/99, 0 échec, 0 ignoré (`webhook-reconcile-contract-target-final.log`). `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent (`rustfmt-check-final.log`, `diff-check-final.log`, `license-state-final.log`). Le garde Cargo retourne `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, SPDX `NOASSERTION`.

Outils : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, builds `--offline --locked`.

## Identité de la source

Hash du fichier entier du worktree après ce lot (le fichier contient également du travail antérieur) :

```text
2248b0803c6f081dec08bb9a4fc779ef2d2d1975c39469e987bb4b905b97470f  tests/webhook_reconcile_contract.rs
```

Les manifestes append-only référencent les sorties et documents du lot et conservent les préfixes existants : `SHA256SUMS`, `manifest-prefix-before.json`, `manifest-check.txt`, `global-manifest-check.txt`.

## Limites

Test de bibliothèque avec `JournalEvent` synthétiques et adaptateur `FixtureApi` en mémoire. Il ne couvre ni ingress receiver, ni processus CLI, ni instance Forgejo native, ni interruption, course concurrente ou état persistant réel. Aucun code d’exécution n’a changé; la suite all-targets complète n’a pas été rejouée. Aucun modèle/AgentOS, service externe, réseau, dépense ou production n’a été utilisé. La mission reste incomplète.
