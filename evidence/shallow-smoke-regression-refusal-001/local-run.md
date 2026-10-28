# Refus d'une régression masquée par un smoke test — lot 001

Date : 25 octobre 2026. Dépôt local : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le worktree était déjà fortement modifié; ce lot n'a touché qu'au test unitaire local de `src/mission_012_host.rs` et aux nouvelles preuves. Aucun code Forgejo, service ou donnée externe n'a été modifié.

## Scénario et résultat

Le candidat clamp correctement les deux extrêmes, mais renvoie `minimum` pour une valeur pourtant comprise dans `[minimum, maximum]`. Le smoke test, qui vérifie seulement les deux valeurs extérieures, réussit. Le même source candidat exécuté par le vrai `run_sandbox_test` contre la fixture d'acceptation complète échoue sur `values_inside_the_range_are_unchanged` avec `left: -2`, `right: 4`. Le hash du source candidat reste identique après vérification.

Le test construit ensuite une preuve synthétique portant `Outcome::Fail` et constate que `validate_merge_preflight` refuse avec `Proof(OutcomeFailed)`, même si l'instantané de PR est autrement mergeable. Contrôle négatif : le champ de preuve a temporairement été muté de `Fail` à `Pass`; le même test échoue parce que le préflight renvoie `Ok(())`. Le fichier source a été restauré byte à byte depuis la sauvegarde, son SHA-256 comparé, puis le test complet du binaire a repassé.

## Commandes et résultats

Versions complètes : [tool-versions.txt](tool-versions.txt). Hash du source de test : [source-hashes.txt](source-hashes.txt). Les sorties brutes sont conservées dans ce dossier.

- `cargo test --offline --locked --bin mission-012-verify-local host::tests::shallow_smoke_green_cannot_authorize_candidate_rejected_by_protected_acceptance -- --exact --nocapture` — 1/1 après restauration, réussite; voir [focused-bin-restored.log](focused-bin-restored.log).
- Le contrôle négatif de mutation `Outcome::Fail` → `Outcome::Pass` — échec attendu du test, qui observe `Ok(())` face au refus attendu; voir [pass-labeled-negative-control.log](pass-labeled-negative-control.log).
- `cargo test --offline --locked --bin mission-012-verify-local` sur la source restaurée et formatée — 18/18; voir [verifier-bin-tests-final.log](verifier-bin-tests-final.log).
- `cargo fmt --all -- --check` — réussite finale; voir [rustfmt-final.log](rustfmt-final.log). Le premier contrôle avait relevé le format de lignes ajoutées, corrigé avant le run final.
- `cargo clippy --offline --locked --bin mission-012-verify-local --tests` — sortie 0, avertissements Clippy existants répartis dans plusieurs modules/cibles; aucun avertissement n'est émis à la ligne du nouveau test. Voir [clippy-final.log](clippy-final.log).
- `cargo metadata --offline --locked --no-deps --format-version 1` — `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, redistribution `not_assessed`; readback dans [cargo-metadata.json](cargo-metadata.json).

Deux sélections initiales de tests ont exécuté zéro cas (`--lib` et un préfixe de module incorrect pour le binaire). Elles sont conservées dans `focused.log` et `focused-bin.log`, ne comptent pas comme résultat; le filtre correct est celui du binaire `mission-012-verify-local` et du module `host::tests`.

## Portée et limites

Démontré : un smoke vert peut manquer une régression sémantique; l'acceptation protégée produit un échec actionnable; un verdict signé `Fail` est refusé au préflight; l'assertion détecte le cas de preuve mal étiquetée `Pass`. L'exécution du candidat passe par le runner Bubblewrap/systemd existant.

Cette preuve utilise un crate temporaire, une clé de signature de test et un instantané de PR synthétique. Elle ne démontre ni un LLM, ni une identité de vérificateur de production, ni un webhook/PR/merge Forgejo réel, ni le comportement après merge. Le préflight testé est une décision de lecture seule, pas l'acte de fusion. Aucun benchmark conventionnel ou mesure de valeur n'est réalisé; le critère général d'achèvement reste ouvert.

Manifeste de ce dossier : [SHA256SUMS](SHA256SUMS).
