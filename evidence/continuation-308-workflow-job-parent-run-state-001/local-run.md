# CONTINUATION-308 — état du workflow job et du run parent

Date : 27 octobre 2026. Worktree : `.runtime/webhook-005-workspace`; HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun checkout, état utilisateur ou source Forgejo n’a été réinitialisé ou modifié.

## Problème et changement

Le readback d’un `workflow_job` lisait déjà le job puis son run parent, mais ne retournait dans l’état canonique que le statut du job. La fixture est volontairement contradictoire : job `success`, run parent `failure`. Un agent consommant la sortie seule ne pouvait pas voir le résultat du workflow.

Le résultat canonique ajoute `parent_workflow_run` avec `run_id`, `index_in_repo`, `state`, `updated_at`, `html_url` et `head_sha`. La source est déclarée `current_forgejo_api`, la fraîcheur `current_api_read`; `independent_verification` et `merge_authorizing` valent tous deux `false`. L’état `canonical_state.state` reste celui du job. Aucun nom de runner, étape, log ou payload événement n’est projeté. Les ressources qui ne sont pas des jobs ont `parent_workflow_run: null`. Les schémas passent à single v14, batch v18 et capabilities v20; la découverte précise la sémantique et le refus sans canonical state lorsque le readback parent échoue.

Le test `workflow_job_readback_uses_current_job_and_parent_run_without_runner_details` avait été exécuté avant le changement d’implémentation et échouait car `canonical_state.parent_workflow_run.state` était nul au lieu de `failure`. La sortie brute de cette exécution préliminaire n’a pas été conservée dans un fichier; le contrat conservé et la relance verte sont vérifiables ci-dessous. Ensuite, le consommateur `webhook_quarantine_cli` a réellement échoué sur une attente single/batch périmée (13/17 au lieu de 14/18). C’était un attendu de test obsolète, pas un défaut du CLI; l’attendu a été corrigé puis le test a passé 5/5.

## Vérifications exécutées

Toutes les commandes Cargo utilisent le lockfile existant, hors ligne, et le même checkout local.

| Vérification | Résultat |
|---|---:|
| `cargo test --locked --offline --bin forgejo-reconcile-local --test webhook_reconcile_contract --test forgejo_reconcile_capabilities_e2e -- --test-threads=1` | 163 tests binaires, 81 contrats, 5 capacités réussis; 2 tests capacités ignorés |
| `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details` | 1 test processus ignoré exécuté puis réussi sous Bubblewrap |
| `cargo test --locked --offline --test reconcile_config_contract --test reconcile_journal_lock_timeout --test webhook_quarantine_cli -- --test-threads=1` | après correction : 4 + 3 + 5 tests réussis |
| `cargo test --locked --offline --all-targets --no-run` | compilation de toutes les cibles réussie; aucune prétention que toutes les cibles aient été exécutées |
| `cargo fmt --all -- --check`; `git diff --check` | réussis après formatage |
| garde Cargo et `python3 -m unittest -v scripts/test_cargo_license_state.py` | garde valide et 12 tests réussis |
| compilation Python en mémoire des 9 scripts/consommateurs touchés | 9 fichiers compilés, aucun `.pyc` écrit |

Les sorties conservées sont `focused-contracts.log`, `isolated-process.log`, `schema-consumers.log` (échec de l’attendu périmé), `schema-consumers-fixed.log`, `all-targets-no-run.log`, plus les logs de formatage, de diff et de garde ajoutés au même dossier. Pour le test processus, le runner a copié `Cargo.toml`, `Cargo.lock`, `src/` et `tests/` dans un répertoire temporaire borné; build et test ont eu lieu sous Bubblewrap `--unshare-net --unshare-pid`, avec registre Cargo hors ligne. La fixture API loopback synthétique reçoit exactement deux GET, dans l’ordre `/actions/jobs/9001` puis `/actions/runs/7001`. Le processus CLI retourne `current_state_read`, le statut job `success`, le run parent `failure` et le SHA parent lié. Les étapes/runner, le corps événementiel et détails privés de fixture n’apparaissent pas dans le résultat. Le journal de livraisons reste byte-identical.

Environnement : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, Bubblewrap `0.9.0`, Linux `7.0.0-1019-nvidia-aarch64-with-glibc2.39`; Cargo.lock SHA-256 `459fef3749a42c989521901dcde61cb26e12ed4206c5d722e149426c2d3a5439`. Le run final après formatage a enregistré source staged SHA-256 `dc1270c58a1e07d19fc1180c4c7a7cf199dcdb0c7ee98e49ed152ac10afc7c8e`, runner SHA-256 `0c3ee651856b02ff11684d732a4a848d043cc4b60bb8c61c69d20b8906c990b5` et registry staged SHA-256 `1f73c6a919948c0af44acac0604f8bf4f6ff4888379605e02d3976f1baea6451`.

## Licence et couverture

`cargo metadata --locked --offline --no-deps --format-version 1` est vérifié par `scripts/verify_cargo_license_state.py` : `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`, export SPDX `NOASSERTION`. Les 12 tests de la garde vérifient aussi que ces champs ne se contredisent pas. Aucun choix de licence n’est fait ici.

Le vérificateur de manifestes du lot 307 a refusé une tentative de mise à jour parce que `docs/checkpoint.md` ne correspond plus au snapshot de source inclus dans cette preuve antérieure. Il s’est arrêté avant écriture; le reçu 307 reste historique et intact. Le présent lot possède son propre hash de source pour le checkpoint courant et étend les index globaux en préservant leurs préfixes.

Le parcours est un test de protocole avec une API HTTP synthétique, `n=1`. Il ne démontre ni un run de workflow Forgejo, ni un job ou runner réel, ni une vérification indépendante, ni une autorisation de fusion. L’état d’Actions reste une observation consultative. Aucun Forgejo live, AgentOS, LLM, RAG ou mémoire n’a été ajouté ou appelé; aucun coût externe n’a été engagé. Cela n’améliore pas le nombre d’exigences satisfaites sur un projet et ne constitue pas un benchmark comparatif.

## Fichiers/symboles pertinents

- `src/webhook/reconcile.rs` — `ReconciledWorkflowRun`, `ReconciledState`, `reconcile_workflow_job`, `reconciled_state_value`.
- `src/bin/forgejo-reconcile-local.rs` — schémas single/batch/capabilities, capacités de découverte.
- `tests/webhook_reconcile_contract.rs` — contrat de distinction job/run parent.
- `tests/webhook_receiver_process_e2e.rs` — test du vrai CLI et fixture loopback.
- `tests/forgejo_reconcile_capabilities_e2e.rs`, `tests/webhook_quarantine_cli.rs` — valeurs du contrat annoncé et consommateurs de schéma.
- `scripts/run_forgejo_mission_delivery_process_e2e.py` — exécution isolée du test ignoré.
- `docs/source-inventory.md`, `docs/checkpoint.md` — inventaire courant et checkpoint.
