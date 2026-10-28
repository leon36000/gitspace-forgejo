# CARGO-LICENSE-NO-SELECTION-007 — état Cargo explicite, sans choix de licence

Date : 25 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

La metadata réelle de `agent-forge-core` rapporte `license=null`, `license_file=null` et `publish=[]`. Elle contient le marqueur projet `metadata.agent-forge.licensing.decision=no_license_selected` avec `redistribution_rights=not_assessed`. Dans `Cargo.toml`, `publish = false` ferme la publication. Aucun SPDX, fichier de licence ni droit de redistribution n’est sélectionné ou inféré.

## Vérifications

- `cargo metadata --locked --offline --format-version 1 --no-deps` : code 0; le readback réduit des champs pertinents est conservé dans `metadata-readback.json`.
- `python3 scripts/verify_cargo_license_state.py` : code 0, `status=verified`.
- `python3 -m unittest -v scripts/test_cargo_license_state.py` : 7/7. Les tests acceptent le statut non choisi et refusent une licence, un `license_file`, une publication activée, des métadonnées absentes/ambiguës ou des droits déclarés évalués.
- SHA-256 des trois sources courantes et versions d’outils : voir `source-sha256.txt` et `tool-versions.txt`.

Ce lot ne modifie pas le manifeste : le contrat demandé est déjà présent et passe ses contrôles. Il résout l’ambiguïté du statut Cargo, pas la question juridique sous-jacente; les droits restent réellement non évalués. Aucun paquet n’a été publié ou préparé pour publication.
