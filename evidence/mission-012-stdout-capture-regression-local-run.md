# MISSION-012 — assertion stable pour l’anti-falsification de sortie

Date : 2026-10-22. Cible : `src/mission_012_host.rs::candidate_cannot_forge_libtest_success_with_print_and_process_exit`.

## Cause

Deux exécutions complètes ont échoué dans cette assertion de test. La fixture candidate écrit un faux résumé libtest puis appelle `std::process::exit(0)` depuis un test d’acceptation. Le runner hôte lance `cargo test` avec la capture libtest par défaut (`src/mission_012_host.rs::run_sandbox_test`, sans `--nocapture`). La sortie capturée du test peut ne pas être vidée si le processus se termine avant que libtest rende son résumé. L’assertion externe exigeait que le texte de l’acceptation rouge apparaisse dans cette sortie du candidat, alors que l’hôte avait correctement renvoyé `verified_acceptance_pass=false` et refusait son diff. Les sorties sont conservées dans `forgejo-preflight-cli-e2e-001-all-targets-initial-failure.log` et `forgejo-preflight-cli-e2e-001-all-targets-stdout-capture-failure.log`.

## Correction limitée au test

Les assertions dépendant du stdout ont été retirées. Le test exige maintenant le code de sortie zéro de la commande attaquante, aucun timeout ni troncature, `approved_candidate_source=false`, `verified_acceptance_pass=false`, puis le refus `candidate_source_contract_violation` au diff. Le test vérifie ainsi la décision fail-closed de l’hôte sans dépendre de la vidange de la sortie capturée. Le code de production n’a pas été modifié.

## Vérification

- Les tests exacts de `mission-012-mcp-host` et `mission-012-verify-local` réussissent; commandes et sorties : `mission-012-stdout-capture-regression-targeted.log`.
- `CARGO_TARGET_DIR=/tmp/webhook-receiver-adapter-025-clean-target cargo test --offline --locked --all-targets` réussit : 24 suites, 211 réussis, 0 échec, 7 ignorés; sortie finale dans `forgejo-preflight-cli-e2e-001-all-targets.log`.
- Clippy all-targets réussit avec les trois catégories de lints préexistantes neutralisées; rustfmt et `git diff --check` réussissent. Le rapport MERGE-003 et son manifeste regroupent ces sorties.

Cette correction stabilise seulement l’assertion du test. Elle ne change ni le processus sandboxé, ni son contrat de sortie, ni la décision de vérification de production.

Une revue statique indépendante confirme que les assertions restantes couvrent le refus du candidat sans dépendre de stdout capturé : [note de revue](mission-012-stdout-capture-regression-review.md).
