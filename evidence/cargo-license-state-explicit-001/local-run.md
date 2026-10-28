# CARGO-LICENSE-STATE-EXPLICIT-001

Date : 25 octobre 2026. Le paquet Rust reste `agent-forge-core` 0.1.0 dans le worktree local dirty basé sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## État Cargo retenu

Le champ Cargo `license` reste absent (`null`) et `license_file` reste absent (`null`); `publish = false` interdit la publication depuis ce manifeste. Les métadonnées privées du paquet indiquent maintenant `decision = "no_license_selected"` au lieu du terme ambigu `unselected`. `redistribution_rights` reste `not_assessed`. Cette valeur n’est ni une licence SPDX, ni une attribution de droits, ni une autorisation de redistribution. Le choix d’une licence demeure réservé au détenteur des droits.

## Preuves

- Avant la modification du garde, `python3 scripts/test_cargo_license_state.py` échouait sur la nouvelle représentation attendue parce que le script exigeait encore exactement `unselected`; la sortie est dans `tests-red.log`.
- Après mise à jour du manifeste, du garde et de ses contrats, `python3 scripts/test_cargo_license_state.py` passe 7/7; sortie dans `tests-green.log`.
- `python3 scripts/verify_cargo_license_state.py` exécute `cargo metadata --locked --offline --format-version 1 --no-deps` et retourne `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `status=verified`; readback dans `metadata-readback.json`.

Le changement ne sélectionne aucune licence et ne conclut rien sur les droits des dépendances. Il maintient l’interdiction de publier le crate. Le test vérifie l’état du manifeste; il n’est pas un avis juridique ni un audit de l’assemblage.

Les versions d’outils et les empreintes des sources sont dans `tool-versions.txt` et `source-sha256.txt`. Le manifeste local `SHA256SUMS` couvre huit artefacts, dont ce rapport et le readback Cargo; il s’exclut lui-même. La vérification `sha256sum -c` est conservée dans `manifest-check.txt`.
