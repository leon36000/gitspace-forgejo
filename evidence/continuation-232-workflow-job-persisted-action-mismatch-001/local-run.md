# CONTINUATION-232 — replay CLI refuse aussi une action incompatible de workflow_job

Date : 2026-10-27. Worktree `webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dépôt dirty.

## Résultat

Le scénario de replay action/statut a été factorisé dans un helper partagé et couvre désormais les deux ressources. Un `workflow_run_blocked` valide (`run_status_changed`) et un `workflow_job_waiting` valide (`job_status_changed`) sont d’abord signés et admis par le vrai receiver. La fixture modifie ensuite seulement l’action persistée (`run_completed` ou `job_completed`) et recalcule le hash de ligne; chaque inbox conserve une chaîne de hash valide.

Le CLI réel refuse ensuite chaque livraison avant réseau avec `inconclusive/invalid_envelope`, `details.field_path=payload.action`, `persistence_state=already_persisted`, zéro lecture amont et champs HMAC faux/non revérifiés. L’écouteur réservé à `127.0.0.1:3130` ne reçoit aucune connexion et le journal demeure identique après le refus.

Deux commandes isolées sous Bubblewrap `--unshare-net --unshare-pid` passent :

- `actual_reconcile_cli_process_refuses_persisted_incompatible_workflow_action_before_api_read` : 1/1, 141 filtrés.
- `actual_reconcile_cli_process_refuses_persisted_incompatible_workflow_job_action_before_api_read` : 1/1, 141 filtrés.

Le premier essai du cas job a échoué à l’admission avec HTTP 400 `payload.repository`: le fixture avait placé `repository` au mauvais niveau. Ce log est conservé dans `isolated-cli-job.log`; le payload a été corrigé selon le contrat existant (`run.repository`) et le rerun passe dans `isolated-cli-job-green.log`. La reprise du cas run après factorisation passe aussi dans `isolated-cli-run-refactor.log`.

## Vérifications et couverture

`cargo fmt --all -- --check` et `git diff --check` passent. Le lanceur Python compile le code hors réseau et exécute uniquement chaque test ignoré demandé. Les changements de ce lot sont limités au test processus et à sa liste de scénarios.

Limites : les inbox modifiées sont des fixtures synthétiques dont le hash est recalculé; cela ne rétablit pas leur HMAC ni ne prouve leur origine. Aucun serveur Forgejo/runner n’est lancé. Aucun agent LLM/AgentOS n’est invoqué; l’absence d’adaptateur utilisable est décrite dans CONTINUATION-230. Aucun gain comparatif n’est mesuré et aucune licence n’est attribuée. Le projet demeure incomplet.
