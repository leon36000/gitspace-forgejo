# MISSION-RAW-REQ-DUPLICATE-JSON-001 — conserver puis refuser une exigence JSON ambiguë

Date : 25 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, sans commit dédié et avec des modifications préexistantes importantes. Sources examinées : `src/bin/forgejo-mission-local.rs`, `src/mission_admission.rs`, `tests/mission_admission_contract.rs`; empreintes et outilchain dans ce dossier.

## Contrat observé

Le client local récupère un document depuis `/{owner}/{repo}/raw/commit/{commit}/{path}` et conserve son corps brut. L’admission désérialise ensuite ce corps vers `Requirements`; un document invalide doit être refusé avant la création d’une claim. Le dérivé Serde de `Requirements` refuse déjà la répétition de ses champs connus. Ce lot ajoute une preuve de contrat, sans prétendre corriger un défaut runtime constaté.

`raw_requirement_read_pins_commit_and_preserves_duplicate_json_fields` lance le vrai lecteur HTTP/curl contre un listener loopback éphémère. Le listener vérifie le chemin incluant le commit intégral et sert deux membres JSON `acceptance` contradictoires. Le test confirme que les octets retournés sont identiques à ceux servis et que le document ne se désérialise pas en `Requirements`.

`duplicate_acceptance_json_field_is_refused_before_generation_allocation` injecte le même document au port d’admission, vérifie l’erreur `mission_requirements_invalid`, un seul read issue et requirement, et un journal d’exécution vide. Cela vérifie le refus avant toute allocation/claim.

## Résultats vérifiés

- `cargo test --offline --locked --bin forgejo-mission-local -- --test-threads=1` : 31 réussis, 0 échec, 0 ignoré.
- `cargo test --offline --locked --test mission_admission_contract -- --test-threads=1` : 15 réussis, 0 échec, 0 ignoré.
- `cargo fmt --all -- --check` : code 0.
- `python3 scripts/verify_cargo_license_state.py` : `verified`; `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.
- `git diff --check` : code 0.

Commandes, sorties, toolchain, HEAD, branche et empreintes des sources sont conservés ici. Le manifeste local couvre tous les artefacts du dossier hors manifeste lui-même et passe `sha256sum -c`.

## Portée et limites

Le test HTTP utilise curl et un serveur synthétique lié à loopback; l’admission contractuelle utilise `FakeSource`. Aucun serveur Forgejo n’est lancé par ce lot. Le test établit le transport exact jusqu’au lecteur local et le refus du décodeur/admission pour un champ dupliqué, pas la protection de toute l’exécution sur toutes les versions Forgejo. Aucun parcours LLM/AgentOS, comparaison contrôlée, coût modèle ou intervention de production n’est démontré. Aucun runtime ou code upstream Forgejo n’a été modifié.

Prochaine action : compléter l’audit des réponses d’exigences brutes en couvrant les statuts et limites de taille/UTF-8, sans répéter les cas de pagination déjà prouvés; reprendre séparément l’analyse scratch du fencing multi-writers LFS. Le projet reste incomplet.
