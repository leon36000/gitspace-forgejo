# CONTINUATION-332 — labels périmés dans les webhooks hors ordre

Date : 2026-10-28. Worktree local : branche `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dépôt fortement modifié. Aucun code de production n’a changé dans cette tranche; le changement ajoute un test de composition receiver/journal/réconciliateur.

## Scénario et résultat

Le test HMAC-signe et soumet deux notifications `issues/edited` pour le même dépôt/issue. La première porte le label périmé `{id:99,name:stale/newer-webhook}`; une seconde livraison d’arrivée tardive porte `{id:3,name:stale/older-webhook}`. Elles sont toutes deux acceptées et journalisées aux séquences 1 et 2. Le readback du `ForgejoCurrentState` fixture fournit l’issue actuelle ouverte et uniquement le label `{id:12,name:canonical/current}`. `reconcile_inbox_events` retourne deux résultats dans l’ordre du journal, chacun avec l’état courant et le label API ID 12; un seul GET est effectué pour le `ResourceRef` partagé. Les payloads d’événement ne sont pas utilisés comme source canonique de labels.

## Commandes et résultats

- `cargo test --offline --locked --test webhook_reconcile_pipeline out_of_order_signed_label_events_use_one_canonical_api_label_snapshot -- --exact --nocapture` — PASS, 1/1; log complet : [pipeline-test.log](pipeline-test.log).
- `cargo fmt --all -- --check` — PASS : [rustfmt.log](rustfmt.log).
- `python3 scripts/verify_cargo_license_state.py` — PASS; aucune licence sélectionnée, publication désactivée : [cargo-license-state.json](cargo-license-state.json).
- `git diff --check` — PASS pour les fichiers suivis : [tracked-diff-check.log](tracked-diff-check.log).
- Runtime Cargo/Rust 1.95.0, Linux aarch64, dépendances hors ligne.

## Portée et limites

Le scénario couvre la véritable validation HTTP/HMAC et l’écriture du journal au niveau bibliothèque, suivies par le vrai batch reconciler avec une fixture du readback Forgejo. Il ne lance ni les binaires receiver/reconciler en sous-processus, ni Forgejo live; le readback est déterministe et aucune concurrence réseau n’est créée. Il ne prouve pas qu’un LLM/AgentOS consomme le résultat, ni PR, merge, vérification indépendante ou gain comparatif. Aucun accès externe ou coût nouveau n’a été utilisé. Le projet reste incomplet.
