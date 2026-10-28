# CONTINUATION-375 — readback courant des labels de pull request dans un batch

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout reste fortement modifié; aucun fichier existant n’a été supprimé, aucun changement staged, aucun commit ni push.

## Pourquoi cette tranche

La projection partagée `current_work_item_labels` prend déjà en charge `Issue` et `PullRequest`, mais les tests intégrés receiver→journal→processus CLI ne couvraient pas les labels PR périmés. Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, confirme dans `services/webhook/notifier.go::IssueChangeLabels` l’action `label_updated` sous le header PR avec un `PullRequestPayload`; `modules/structs/pull.go::PullRequest.Labels` expose la liste dans le payload. Les deux fichiers observés correspondent à HEAD dans le scratch upstream, bien que ce scratch soit sale pour d’autres POC. Leurs hashes sont consignés dans `source-sha256.json`.

## Résultats observés

Le test processus `actual_reconcile_cli_process_projects_current_pull_request_labels_for_batch` fait accepter par le vrai receiver deux deliveries signées `pull_request/label_updated` dont les snapshots divergent (IDs 99 et 3). Le vrai CLI traite le journal avec une seule lecture anonyme `GET /pulls/54`. La réponse API courante contient les labels non triés IDs 20 puis 3; chaque résultat expose les labels API courants triés 3 puis 20, avec source `current_forgejo_resource_api`, fraîcheur `current_api_read` et confiance `untrusted_user_content`. Aucun texte de label webhook périmé ne paraît dans stdout et le journal reste byte-identical.

Le test `actual_reconcile_cli_process_refuses_malformed_current_pull_request_labels` reçoit une livraison similaire mais la réponse API courante contient `labels.id=0`. Le CLI sort 1 et retourne `inconclusive/invalid_current_state`, `details.field=labels`, une lecture API et aucune `canonical_state`. Aucun label périmé ou mal formé ne fuite dans stdout; l’inbox reste byte-identical.

Les deux tests passent 1/1 chacun via `scripts/run_webhook_reconcile_cli_e2e.py` sous Bubblewrap `--unshare-net --unshare-pid`. L’adaptateur utilise uniquement une fixture HTTP loopback; aucun credential n’est envoyé. Le contrat `webhook_reconcile_contract` passe 98/98 et `webhook_reconcile_pipeline` 6/6.

## Contrôles et licence

`cargo fmt --all -- --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` et `git diff --check` passent. Le premier contrôle rustfmt avait signalé deux lignes dans le nouveau test; `rustfmt` les a formatées et le contrôle final passe (`rustfmt.log` initial et `rustfmt-final.log`). Les tests de politique Cargo passent 12/12. Readback `cargo metadata --locked --offline --format-version 1 --no-deps` : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, SPDX `NOASSERTION`. `Cargo.toml` n’est pas modifié par cette tranche et aucune licence n’est choisie.

## Portée et suite

La vérification traverse un receiver et un CLI réels, mais ne lance pas une instance Forgejo pour émettre le webhook; sa forme a été confrontée au code source officiel v17.0.0. Le lot ajoute des tests et l’allowlist du runner, sans changement de logique runtime. Il ne prouve pas la sémantique labels absents/`null`/vide pour chaque endpoint au niveau processus, une stabilité après le GET, un agent LLM/AgentOS, une fusion, ni un gain comparatif. Le dépôt GitLab PC4 n’a pas été contacté dans cette tranche; le réseau reste inaccessible depuis PC6 et aucun credential utilisable limité au projet cible n’a été employé. Aucun secret, modèle ou service externe n’est utilisé.

La prochaine tranche locale doit continuer sur une lacune non répétée de réconciliation PR ou un test d’émission native si le Forgejo jetable local est disponible; le projet reste incomplet.
