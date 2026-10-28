# CONTINUATION-387 — capacités v8 du receiver-process

Date : 28 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucune source runtime n’a été modifiée dans ce lot.

## Preuve exécutée

La commande exacte est dans `verification-commands.txt`; sortie observée dans `test-output.txt` :

```text
running 1 test
test capabilities_endpoint_describes_the_versioned_webhook_contract_without_mutating_inbox ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 182 filtered out
```

Le test lance `forgejo-webhook-local`, se connecte en loopback à son vrai endpoint `/forgejo/capabilities`, confirme le schéma supérieur `8`, les contrats v1–v8 et notamment `event_contracts_v8`, ainsi que les règles d’idempotence annoncées. Il vérifie l’absence de champs `secret` et `credential`, refuse POST avec 405, puis compare l’inbox avant/après pour s’assurer que la découverte ne la modifie pas.

Environnement Rust observé : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`. Cargo utilise `--offline --locked`.

## Portée

Ce résultat établit que le processus receiver local expose le contrat v8 cohérent avec le handler et le CLI. Il n’exerce pas un serveur Forgejo upstream, une émission native de webhook, un provider AgentOS/LLM, une PR ou fusion réelle, ni un benchmark. Il n’ajoute ni dépendance ni choix de licence. Voir aussi [CONTINUATION-386](../continuation-386-issue-comment-is-pull-required-001/local-run.md) pour le défaut corrigé et les tests positifs/négatifs du payload.
