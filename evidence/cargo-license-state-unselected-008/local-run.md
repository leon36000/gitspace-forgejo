# CARGO-LICENSE-STATE-UNSELECTED-008 — état de licence Cargo relu sans choix

Date : 26 octobre 2026. `cargo metadata --format-version 1 --no-deps` a été relu pour le paquet unique `agent-forge-core 0.1.0`; `license=null`, `license_file=null`, `publish=[]`. Les métadonnées de revue déclarent `decision=no_license_selected`, `redistribution_rights=not_assessed` et `spdx_package_license_declared=NOASSERTION`. `python3 scripts/verify_cargo_license_state.py` renvoie `status=verified`; `python3 -m unittest -v scripts.test_cargo_license_state` passe 9/9.

Cette vérification établit uniquement que le manifeste exprime explicitement l’absence de choix et garde la publication Cargo désactivée. Elle n’évalue pas les droits de redistribution de GitSpace, Forgejo ou des dépendances, et ne sélectionne aucune licence pour le produit. Manifest, script, test, JSON de readback et log sont snapshotés dans `source/`, `cargo-metadata.json`, `verify.json` et `license-tests.log`.
