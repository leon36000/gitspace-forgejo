# MISSION-RAW-REQ-RESPONSE-BOUND-001 — refuser un téléchargement d’exigence hors borne comme erreur terminale

Date : 25 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, sans commit dédié et fortement modifié avant ce lot. Sources et toolchain exactes : `source-sha256.txt` et `tool-versions.txt`.

## Défaut reproduit et correction

`LocalLoopbackForgejoSource::get` limitait curl à 1 MiB. En cas de réponse HTTP 200 annonçant `Content-Length: 1 048 577`, curl 8.5.0 sortait avec le code 63 (fichier trop volumineux). Le helper convertissait tout code curl non nul en `SourceFailureV1::Unavailable`; l’erreur remontait alors comme réessayable, bien qu’une réponse dépassant la borne soit invalide et ne se corrige pas par répétition.

Le test `oversized_raw_requirement_response_is_non_retryable_invalid_response` traverse le vrai lecteur et le vrai binaire curl contre un listener loopback. Rouge, il recevait `Unavailable` au lieu de `InvalidResponse` (exit 101). Le helper classe maintenant le code curl 63 comme `InvalidResponse`. L’erreur structurée est non réessayable, indique la limite de 1 MiB dans son action et expose `max_response_bytes=1048576`; le test vérifie ces trois champs.

Le lot conserve aussi le contrat établi juste avant : le corps `raw/commit/{commit}/requirements/REQ-001.json` est transmis sans normalisation, puis un document avec membres `acceptance` dupliqués est refusé avant claim. Voir [MISSION-RAW-REQ-DUPLICATE-JSON-001](../mission-requirement-duplicate-json-field-001/local-run.md).

## Résultats vérifiés

- Test rouge ciblé : échec attendu, exit 101, `Unavailable` observé au lieu de `InvalidResponse`.
- Test vert ciblé : 1 réussi, 0 échec; refuse l’excès, action non réessayable et limite explicite.
- `cargo test --offline --locked --bin forgejo-mission-local -- --test-threads=1` : 32 réussis, 0 échec.
- `cargo test --offline --locked --test mission_admission_contract -- --test-threads=1` : 15 réussis, 0 échec.
- `cargo test --offline --locked --test webhook_mission_admission_contract -- --test-threads=1` : 13 réussis, 0 échec. Ce contrôle vérifie l’interface d’admission/reconciliation webhook adjacente; son source reste une fixture.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : 496 réussis, 0 échec, 55 ignorés sur 34 groupes.
- `cargo fmt --all -- --check` et `git diff --check` : code 0.
- `python3 scripts/verify_cargo_license_state.py` : `verified`, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.

Les sorties, codes, commandes, versions, HEAD, branche et SHA source sont conservés dans ce dossier. Le manifeste local couvre tous les artefacts hors manifeste lui-même et passe `sha256sum -c`; les hashes courants sont ajoutés en fin du manifeste global sans réécrire l’historique.

## Portée et limites

La réponse HTTP est une fixture loopback; le client curl et la classification du binaire sont réels, mais Forgejo n’est pas lancé. Aucun chemin de merge, writer Forgejo, LLM/AgentOS, service externe, dépense ou production n’est exercé. Les 55 tests ignorés n’ont pas tourné dans la commande all-targets; des E2E dédiées ont leurs runners d’isolation séparés. Le test prouve la classification de cette limite, pas l’ensemble des statuts/encodages de toutes les routes.

Le Cargo manifest reste volontairement sans champ `license` ni `license_file`; `publish=false` et `no_license_selected` demeurent explicites. Aucune licence ni droit de redistribution n’est sélectionné. Le parcours LLM et le benchmark contrôlé restent non démontrés; le projet est incomplet.

Prochaine action : tester la réponse raw invalide en UTF-8 et les statuts terminal/transitoire du même lecteur, sans rejouer la pagination ni les parcours verts; poursuivre ensuite les refus composés de réconciliation qui manquent encore et garder séparée l’analyse de fence LFS dans le scratch.
