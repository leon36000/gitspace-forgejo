# CONTINUATION-083 — borner l’URL Web d’un job à son run

Date : 26 octobre 2026. Produit : worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà fortement modifié. Ce lot ajoute un test de contrat et actualise la documentation et les reçus; aucun reset, commit, changement de production ou appel externe.

## Invariant et source

Le test couvre le lien `html_url` retourné pour le job Actions courant : le lien doit être une URL canonique de job à l’intérieur du run lu, sans hôte étranger, route de logs ni paramètre de redirection. Forgejo v17.0.0 construit ce lien dans `models/actions/run_job.go::HTMLURL` à partir de l’index du job, qui commence à zéro. Source officielle examinée au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; détails et SHA-256 des fichiers amont : [CONTINUATION-080](../continuation-080-workflow-job-readback-001/local-run.md) et son `source-sha256.txt`. Aucun code Forgejo n’est copié.

## Résultats

Le test `workflow_job_readback_accepts_zero_based_first_job_and_rejects_unscoped_urls` utilise une API fixture. Il accepte l’URL du premier job `/actions/runs/41/jobs/0/attempt/1`. Il rejette six variantes : hôte étranger, tentative zéro, index avec zéro initial, tentative avec zéro initial, suffixe `/logs` et query de redirection. Pour chaque variante refusée, le readback retourne `InvalidCurrentStateField("html_url")` et ne projette pas d’état. Les deux lectures attendues (job et run parent) sont vérifiées. Un test processus distinct traverse le receiver réel et le vrai CLI : l’URL externe vient de l’API fixture, le CLI répond `inconclusive/invalid_current_state`, n’émet aucun `canonical_state`, ne divulgue pas le domaine externe, laisse le journal inchangé et ne demande pas de logs (`process-e2e.log`). Le test est lancé dans Bubblewrap avec les namespaces réseau et PID isolés.

Commandes et reçus :

- `cargo test --offline --locked --test webhook_reconcile_contract workflow_job -- --nocapture` : 3 réussis, 0 échec, 49 filtrés (`focused.log`).
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_unscoped_workflow_job_url` : 1 réussi, 0 échec, 98 filtrés; processus réel sous Bubblewrap (`process-e2e.log`).
- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 34 exécutables, 597 réussis, 0 échec, 91 ignorés (`all-targets-final.log`, `test-summary.txt`).
- `cargo fmt --all -- --check` et `git diff --check` passent (`rustfmt-final.log`, `diff-check-final.log`).
- La garde d’état Cargo et ses tests passent : 9/9. Aucune licence n’est sélectionnée (`license=null`, `license_file=null`, `publish=[]`, droits `not_assessed`, SPDX `NOASSERTION`).

Le test ne modifie pas le comportement du produit; il vérifie la validation déjà présente au niveau contrat. Il s’agit d’un test fixture, pas d’un parcours vers une instance Forgejo live. Taille de preuve : un lien valide et six formes invalides; aucune route HTML live, aucun runner Actions réel et aucun workflow exécuté.

## Limites

Aucune conclusion de gain comparatif, de sécurité générale ou de correction d’un job n’en découle. Aucun LLM, AgentOS, RAG, mémoire, secret, service externe, dépense ou environnement de production n’a été utilisé. La suite passée ne ferme pas les critères globaux de la mission : benchmark apparié, intégration réelle d’un runtime autorisé, inventaire/licences exhaustifs et preuves restantes de refus/reprise. Voir [checkpoint](../../docs/checkpoint.md), [architecture](../../docs/architecture.md), [inventaire](../../docs/source-inventory.md) et [matrice des refus](../../docs/refusal-evidence-matrix.md).
