# CONTINUATION-085 — refuser un job attaché à un autre run au readback processus

Date : 26 octobre 2026. Produit : worktree /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace, branche local/WEBHOOK-005-canonical-links, HEAD de base f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0. Le checkout demeure fortement modifié. Ce lot ajoute un test processus; il ne modifie pas le code runtime ni Cargo.

## Scénario et résultat

Le receiver accepte un webhook HMAC cohérent où job 9001 et run parent annoncent tous deux le run ID 7001. À la lecture canonique, la fixture API du job répond avec run_id=7002. Le vrai CLI Rust retourne inconclusive/resource_mismatch, sans canonical_state; il n’effectue qu’un GET du job, ne lit pas le run parent, ne sollicite pas /logs, ne révèle pas un marqueur privé de la réponse et ne réécrit pas le journal durable.

Le test actual_reconcile_cli_process_refuses_job_attached_to_another_run_before_parent_read passe à travers le vrai receiver puis le vrai binaire forgejo-reconcile-local, dans Bubblewrap avec namespaces réseau/PID séparés, environnement effacé et source en lecture seule. Il complète le test de contrat workflow_job_readback_refuses_job_from_another_run_before_parent_read par une observation au niveau du processus. Ce test utilise une API loopback fixture, pas une instance Forgejo live.

## Vérifications

- python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_job_attached_to_another_run_before_parent_read : 1 réussi, 0 échec, 99 filtrés (process-e2e.log).
- cargo test --offline --locked --all-targets -- --test-threads=1 --quiet : 34 exécutables, 597 réussis, 0 échec, 92 ignorés (all-targets.log, test-summary.txt). Les tests processus ciblés ignorés dans l’exécution générale sont exécutés séparément par leur driver.
- cargo fmt --all -- --check, git diff --check et python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py passent.

La dernière preuve de politique Cargo est CONTINUATION-083 (../continuation-083-workflow-job-link-scope-001/cargo-license-verify-final.json) : license=null, license_file=null, publish=[], décision no_license_selected, droits not_assessed, marqueur NOASSERTION. Ce lot ne change pas ces métadonnées et ne relance pas leurs neuf tests.

## Portée et limites

Ce refus est un résultat au processus sur le contrat API configuré, pas une validation de la réponse Forgejo réelle, de l’émission native, d’un runner Actions ni de tous les champs possibles. Le statut du job demeure consultatif; aucun succès Actions ne devient une preuve indépendante ou une autorisation de fusion. Aucun LLM, AgentOS, RAG, mémoire, coût, service externe, secret ou environnement de production n’est utilisé. Le benchmark contrôlé et l’intégration d’un runtime agent restent ouverts.
