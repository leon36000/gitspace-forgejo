# CONTINUATION-028 — liste complète dans le refus `unsupported_event`

Date : 26 octobre 2026. Checkout local : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; aucun changement préexistant n’a été réinitialisé ou supprimé.

## Écart reproduit et correction

Le receiver refusait correctement un en-tête non pris en charge avec HTTP 422, `unsupported_event`, `retryable=false` et aucun append au journal. Son texte d’action énumérait manuellement les événements, mais omettait `create`, bien que celui-ci soit dans l’allowlist. Cette omission rendait la consigne erronée pour l’opérateur.

`src/webhook/http.rs` génère maintenant l’énumération avec `SUPPORTED_EVENTS.join(", ")`. Cette constante est la vue publique dérivée de `SUPPORTED_EVENT_CONTRACTS` (CONTINUATION-027). Le code de refus reste inchangé; le test confirme aussi que le journal reste sans mutation.

## TDD et vérification

Le test existant `unsupported_event_is_rejected_without_journal_side_effect` a été renforcé pour exiger la liste complète retournée dans `action`, puis exécuté avant la correction. Il a échoué sur l’ancien texte et a affiché la liste qui omettait `create` (`red.log`). Après le changement, le test passe 1/1 (`green.log`) et confirme que le journal reste vide.

- `cargo test --locked --offline --test webhook_handler_contract unsupported_event_is_rejected_without_journal_side_effect -- --exact --nocapture` — rouge avant la correction, vert 1/1 après.
- `cargo test --locked --offline --all-targets -- --test-threads=1` — 553 réussis, 0 échec, 76 ignorés, 34 groupes.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` — réussis.
- `cargo metadata --format-version 1 --locked --offline --no-deps` — `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`.

Toolchain : `rustc 1.95.0 (59807616e 2026-04-14)` et `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`. Les sorties des commandes et les empreintes source sont archivées dans ce dossier.

## Portée et limites

Ce test couvre le handler local et le refus avant append, pas une instance Forgejo live. La tranche ne change pas l’admission ou la réconciliation et n’ajoute aucun événement. Le suffixe d’explication reste statique. Aucun AgentOS/LLM, coût nouveau, service externe ou benchmark n’a été utilisé. Cargo ne sélectionne aucune licence; les droits de redistribution restent `not_assessed`. Le projet demeure incomplet.
