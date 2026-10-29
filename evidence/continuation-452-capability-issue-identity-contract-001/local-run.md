# CONTINUATION-452 — contrat de capacités pour l’identité partielle d’issue

Date : 2026-10-28 (America/Montreal). Worktree PC6 : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, arbre déjà fortement modifié. Lot limité à ce worktree, sans accès GitLab/courriel, commit, publication, suppression ni changement de production.

## Écart corrigé

CONTINUATION-451 a resserré `validate_current_issue_repository`, mais `--capabilities` annonçait encore que tout champ d’identité isolé suffisait. C’était faux pour `repository.name` seul ou `repository.owner` seul. Le test de contrat modifié en premier a reproduit le décalage : la commande `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --exact --nocapture` a échoué comme attendu, l’API retournant le schéma 31 au lieu du 32 demandé par le nouveau contrat. L’extrait de sortie est conservé dans `red-observed.txt`.

Le document de découverte publie maintenant les identités complètes admises : `full_name` seul, ou le couple `owner`+`name`. Chaque champ d’identité présent est vérifié contre la cible; une moitié de `owner`/`name` est invalide même si `full_name` est aussi fourni. `repository` absent ou nul reste permis pour une réponse API partielle. La version de capacités passe à 32 / `forgejo-reconcile-capabilities-v32`; le validateur d’exécution n’a pas changé dans ce lot.

## Vérifications

- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e`: 5 réussis, 0 échec, 2 ignorés. Les deux ignorés requièrent le port loopback fixe 3130 et sont exclus de cette découverte de capacités.
- `cargo test --locked --offline --bin forgejo-reconcile-local capabilities_list_exact_merge_target_profiles -- --nocapture`: 1 réussi, 0 échec.
- `cargo test --locked --offline --all-targets -- --test-threads=1`: 39 cibles, 789 réussis, 0 échec, 212 ignorés; sortie dans `all-targets.log`. Les ignorés ne sont pas comptés comme exécutés.
- `cargo fmt --all -- --check` et `git diff --check`: passent; sorties dans `rustfmt.log` et `diff-check.log`.
- Vérificateur des liens des cinq documents canoniques : 1 670 liens locaux contrôlés, aucune cible manquante; résultat dans `doc-links.json`.
- `python3 -B scripts/verify_cargo_license_state.py`: passe. Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, SPDX `NOASSERTION`; résultat conservé dans `cargo-license-state.json`.
- Les manifestes append-only worktree et projet passent : 14 066 et 9 235 chemins uniques vérifiés, sans chemin absent, divergent, courant non haché ou dangereux. Le manifeste worktree garde une unique entrée historique legacy non hachée, signalée par le vérificateur.

Environnement : Linux aarch64, `rustc 1.95.0`, `cargo 1.95.0`, Python 3.12.3. Commandes exactes : `commands.txt`; hashes sources : `source-sha256.txt`.

## Portée et limites

La preuve exerce le vrai binaire local pour la commande statique `--capabilities` et son contrat sérialisé. Elle ne contacte pas Forgejo, n’exerce pas une réponse live, et ne prouve ni la justesse de la réconciliation sur un serveur, ni une frontière d’autorité, ni un parcours LLM/AgentOS, ni un gain comparatif. Les deux tests ignorés de la cible n’ont pas été exécutés. Cargo reste volontairement sans licence choisie. Aucun code Forgejo/GitSpace n’est copié.

La mission produit reste incomplète; ce lot rend seulement la découverte fidèle au refus déjà exécuté.
