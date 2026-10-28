# CONTINUATION-396 — quarantaine CLI d’un tag `delete` déjà journalisé

Date : 28 octobre 2026. Worktree local `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà sale et le reste du travail a été préservé; aucun commit ni staging.

## But

Après CONT-395, le processus receiver refusait un tag `delete` avant journaling, mais un ancien journal pouvait déjà contenir ce genre d’événement. Le test existant `real_cli_quarantines_persisted_unsupported_tag_with_field_reason` ne couvrait que `create`. Le nouveau test lance le vrai binaire `forgejo-reconcile-local` via `CARGO_BIN_EXE_forgejo-reconcile-local` sur un enregistrement `delete` tag synthétique écrit par le helper de test.

Le résultat CLI affirme `delivery_quarantined`, `unsupported_event_payload_field`, champ `ref_type`, `canonical_state_created=false` et `upstream_read_attempts=0`. Le replay confirme deux lignes durables : la source originale et sa disposition. Le payload source reprend la forme d’un tag delete; il est déjà dans le journal et ne traverse pas l’ingress courant.

## Commandes et résultats

```text
cargo test --offline --locked --test webhook_quarantine_cli real_cli_quarantines_persisted_unsupported_tag_delete_with_field_reason -- --exact --nocapture
cargo test --offline --locked --test webhook_quarantine_cli -- --test-threads=1
```

Le test ciblé passe 1/1. Toute la cible `webhook_quarantine_cli` passe 7/7, 0 échec, 0 ignoré; logs : `quarantine-tag-delete-cli.log` et `quarantine-cli-target.log`. Les deux invoquent le vrai CLI local via le processus de test, avec inbox temporaire privée; l’action de quarantaine n’effectue aucune lecture upstream.

Vérifications : `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent; logs disponibles dans ce dossier. Le vérificateur confirme `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, SPDX `NOASSERTION`. Aucun choix de licence produit n’a été fait.

Outils : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`; les builds Cargo sont `--offline --locked`.

## Identité de la source

Cette empreinte porte sur le fichier entier après le lot, qui contient aussi des changements antérieurs de ce worktree fortement modifié :

```text
acb8678e236fe6e872c4303890a0a4d8d4fab515bdb94ceae4a224a7acf0fc7f  tests/webhook_quarantine_cli.rs
```

Les manifestes du lot lient le rapport, logs, scripts et documents, tout en préservant les préfixes des index locaux/worktree/projet : `SHA256SUMS`, `manifest-prefix-before.json`, `manifest-check.txt`, `global-manifest-check.txt`.

## Portée et limites

La source d’événement est une ligne d’inbox synthétique créée par le helper; le test établit la classification et la disposition du CLI sur un journal persistant local, pas une migration depuis une ancienne version réelle, une livraison native de Forgejo, ni l’ensemble du parcours receiver→reconciliation. Aucun code d’exécution ne change; la suite complète all-targets n’a pas été rejouée. Aucun LLM/AgentOS, réseau externe, service externe, dépense ou production. La mission reste incomplète.
