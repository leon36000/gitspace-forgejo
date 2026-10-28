# CONTINUATION-235 — refus des membres JSON dupliqués dans le readback Actions

Date : 27 octobre 2026. Worktree local, déjà modifié et préservé; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Résultat

Le test de processus `actual_reconcile_cli_process_refuses_duplicate_current_workflow_status_before_projection` admet d’abord un `workflow_run_success` signé et valide par le vrai receiver. Le CLI courant envoie ensuite un GET sur `/api/v1/repos/{owner}/{repository}/actions/runs/7001`; une fixture loopback répond `200` avec deux membres JSON `status`, `failure` puis `success`.

Le parseur de réponse partagé `parse_curl_api_output` appelle `strict_json::parse_value_rejecting_duplicate_members`. Le processus retourne `inconclusive/upstream_invalid_response`, `retryable=false`, un seul readback et aucun `canonical_state`. Il ne divulgue pas le marqueur inséré dans la réponse, ne transmet pas `Authorization` à la fixture et laisse l’inbox byte-identical. Le test isolé passe 1/1 avec 144 tests filtrés dans Bubblewrap `--unshare-net --unshare-pid`.

La première compilation du nouveau test a échoué avant son exécution à cause d’accolades non échappées dans une chaîne `format!`. L’erreur est conservée dans `isolated-process-initial-build-failure.log`. La fixture a été corrigée en JSON brut avec marqueurs de substitution; le test corrigé passe dans `isolated-process.log`. C’est un défaut de test, pas un échec observé du comportement du CLI.

## Changements et vérifications

La logique de production n’a pas changé : le parseur strict existait et s’appliquait au JSON HTTP courant. Ce lot ajoute le test processus, une fixture HTTP à corps brut et son inscription dans le runner isolé; la matrice et le checkpoint relient désormais cette preuve.

`cargo fmt --all`, `cargo fmt --all -- --check`, `git diff --check` et la compilation syntaxique Python du runner passent. Le garde de licence et le readback Cargo hors réseau confirment `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`, `package.metadata.agent-forge.licensing.decision=no_license_selected`, `redistribution_rights=not_assessed` et `spdx_package_license_declared=NOASSERTION`. Aucun choix de licence, droit de redistribution ou publication n’est déclaré.

Versions, commandes, journaux, empreintes sources et manifestes sont dans ce dossier. Les manifestes locaux, worktree et projet sont append-only; le helper a vérifié les préfixes précédents et les entrées courantes.

## Portée et limites

La réponse HTTP vient d’une fixture loopback; aucun serveur Forgejo ni runner Actions n’est utilisé. Le test couvre un champ `status` dupliqué dans le readback d’un run, pas tous les champs, toutes les routes, toutes les permutations JSON ou les réponses de toutes les versions. Il ne constitue ni vérification de workflow, ni verdict de CI, ni preuve de correction logicielle. Aucun LLM, AgentOS, RAG, mémoire, coût fournisseur, production ou service externe n’est impliqué.

La comparaison appariée reste manquante; l’interface AgentOS/modèle utilisable et autorisée n’est pas constatée; l’autorité atomique sur tous les writers Forgejo n’est pas résolue; les droits de redistribution restent non évalués. La mission demeure incomplète.
