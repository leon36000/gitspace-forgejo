# CONTINUATION-029 — renvoi du refus vers le contrat découvrable

Date : 26 octobre 2026. Checkout local : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree est fortement modifié; les fichiers et artefacts existants ont été préservés.

## Écart et changement

Le lot 028 avait rendu l’allowlist d’`unsupported_event` dynamique, mais le suffixe du message répétait encore les contraintes détaillées de `push`, `delete`, `issue_comment` et des revues. Ces descriptions pouvaient diverger de `SUPPORTED_EVENT_CONTRACTS` et de leur publication HTTP.

Le message conserve la liste générée par `SUPPORTED_EVENTS`, puis indique de consulter l’entrée `webhook_ingest.event_contracts` de `GET /forgejo/capabilities`. Il n’embarque plus de seconde prose sur les formes de payload. HTTP 422, code `unsupported_event`, refus non réessayable et absence d’append restent inchangés.

## TDD et vérification

Le test `unsupported_event_is_rejected_without_journal_side_effect` a été étendu pour exiger le nom de champ et le chemin HTTP en plus de la liste complète. Avant le changement, le test a échoué précisément parce que le message ne pointait pas vers le contrat (`red.log`). Après le changement, il passe 1/1 (`green.log`), conserve la liste complète et constate un journal vide.

- Test ciblé : `cargo test --locked --offline --test webhook_handler_contract unsupported_event_is_rejected_without_journal_side_effect -- --exact --nocapture` — rouge avant, vert 1/1 après.
- Suite complète : `cargo test --locked --offline --all-targets -- --test-threads=1` — 553 réussis, 0 échec, 76 ignorés, 34 groupes.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` — réussis.
- `cargo metadata --format-version 1 --locked --offline --no-deps` — `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`.

Toolchain : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`. Sorties brutes, readback metadata, versions et empreintes sont dans ce dossier.

## Limites

Le handler et la capacité sont testés localement; aucun hook d’une instance Forgejo n’est configuré ou observé par cette tranche. Le contrat de capacités est statique, et ne prouve pas que le serveur distant émet les événements ou qu’un hook est configuré selon ses contraintes. Aucun AgentOS/LLM, coût nouveau, service externe ou benchmark n’a été utilisé. Cargo ne sélectionne pas de licence et les droits de redistribution restent `not_assessed`. Le projet demeure incomplet.
