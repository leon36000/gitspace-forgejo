# CONTINUATION-084 — valider au processus l’URL du premier job Forgejo

Date : 26 octobre 2026. Produit : worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt reste fortement modifié. Ce lot ajuste une donnée de test dans le test processus existant; aucun code runtime, manifeste Cargo, service ou état de production n’est modifié.

## Résultat

Le test `actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details` utilisait auparavant `/jobs/1/attempt/2`. La fixture est maintenant le premier job, `/jobs/0/attempt/1`. Le scénario traverse le receiver HMAC Rust et le binaire réel `forgejo-reconcile-local`; une fixture HTTP loopback renvoie l’objet job et son run parent. Il accepte la livraison, lit le job puis le parent, retourne le lien canonique du premier job, et conserve le journal sans modification. Le processus du test est exécuté sous Bubblewrap avec namespaces réseau et PID séparés, environnement effacé et source en lecture seule. Reçu complet : `process-e2e.log`.

L’index à zéro vient de Forgejo v17.0.0 `models/actions/run_job.go::HTMLURL`, révisé au commit `0d791421f85ca13a63b665053d81ea765fbbc571). Les fichiers sources et leurs SHA-256 sont déjà consignés dans [CONTINUATION-080](../continuation-080-workflow-job-readback-001/local-run.md); CONTINUATION-083 fournit les cas contrat d’URL valide/invalide. Aucun code Forgejo n’est copié.

## Vérifications

- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details` : 1 réussi, 0 échec, 98 filtrés (`process-e2e.log`).
- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 34 exécutables, 597 réussis, 0 échec, 91 ignorés (`all-targets.log`, `test-summary.txt`). Les tests processus ignorés du sandbox sont lancés séparément par leur driver.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent.

L’état Cargo reste sans licence choisie; la garde courante la plus récente est le reçu [CONTINUATION-083](../continuation-083-workflow-job-link-scope-001/cargo-license-verify-final.json). Ce lot ne réexécute pas les neuf tests de politique et ne change pas les métadonnées. Aucun LLM, AgentOS, RAG, mémoire, runner Forgejo Actions, dépense, secret externe ou service externe n’est utilisé.

## Limites et suite

Le serveur API est une fixture, pas une instance Forgejo live. Le test établit que le receiver et le CLI courants acceptent la forme canonique à index zéro; il ne vérifie ni la navigation Web rendue, ni l’émission native par Forgejo, ni l’état d’un job réel. `workflow_job_success` reste une observation non indépendante et ne peut autoriser une fusion.

Le parcours et la comparaison contrôlée restent incomplets. Continuer par le rapprochement des versions/licences/périmètres dans l’inventaire et une lacune de refus encore sans preuve intégrée; conserver le benchmark en attente d’un runtime et d’un budget autorisés.
