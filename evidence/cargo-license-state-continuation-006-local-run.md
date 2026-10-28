# Cargo license state — readback 006

Date : 24 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Résultat

La metadata actuelle du package `agent-forge-core` a `license=null`, `license_file=null`, `publish=[]`. Le garde retourne `license_decision=unselected`, `redistribution_rights=not_assessed`, `status=verified`. `Cargo.toml` garde `publish = false` et ne contient ni champ de licence ni fichier de licence. Aucun choix ou droit de redistribution n’est inféré; aucun fichier `LICENSE` ou appel de publication n’a été ajouté/exécuté.

## Vérifications exécutées

- `python3 scripts/verify_cargo_license_state.py` : état attendu relu depuis `cargo metadata --locked --offline --no-deps`; sortie exacte dans [verify](cargo-license-state-continuation-006-verify.json).
- `python3 -m unittest -v scripts/test_cargo_license_state.py` : **5/5**; les tests refusent un identifiant, un fichier, la publication activée et des métadonnées absentes ou ambiguës. Journal : [tests](cargo-license-state-continuation-006-tests.log).

Cette vérification confirme que l’incertitude reste explicitement non résolue par une fausse licence. La décision du détenteur des droits et l’évaluation de redistribution restent nécessaires avant toute diffusion.
