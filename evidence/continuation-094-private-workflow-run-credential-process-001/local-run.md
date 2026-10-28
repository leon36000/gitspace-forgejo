# CONTINUATION-094 — readback authentifié d’un `workflow_run` privé

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree fortement modifié.

## Scénario et résultat

Le vrai receiver accepte HTTP 202 pour un `workflow_run_success` HMAC lié au dépôt cible. Le CLI prend un jeton **synthétique** dans un pipe anonyme fourni par le processus hôte (`--host-credential-fd`). Il lit ensuite l’état courant via le GET Actions autorisé par l’adaptateur. Le serveur HTTP loopback du test vérifie le header exact `Authorization: token <jeton>` et ne répond 200 qu’en cas d’égalité. Une seule requête est observée, sur `/api/v1/repos/{owner}/{repo}/actions/runs/7001`; aucun chemin Web, logs ou write n’est appelé. Le CLI retourne `current_state_read` avec l’état courant, sans exposer le jeton ni les marqueurs `event_payload`/API dans stdout/stderr. Le journal HMAC reste octet pour octet inchangé.

Test : `actual_reconcile_cli_process_reads_private_workflow_run_with_host_credential_fd`. Commande :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_private_workflow_run_with_host_credential_fd
```

Le driver compile hors ligne puis exécute le receiver/CLI de test sous Bubblewrap avec namespaces réseau/PID isolés, environnement effacé et sources read-only. Le run ciblé passe 1/1, 0 échec, 106 filtrés.

## Vérifications

- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 34 exécutables, 597 réussis, 0 échec, 99 ignorés.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent.
- `python3 -m unittest scripts.test_cargo_license_state -v` : 9/9.
- `python3 scripts/verify_cargo_license_state.py` confirme `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`, `status=verified`.
- Versions et hashes des fichiers source dans `tool-versions.txt` et `source-sha256.txt`; journaux des commandes dans ce dossier.

## Portée et limites

C’est un parcours processus du code actuel avec une fixture API locale. Le jeton est synthétique; aucun secret réel, serveur Forgejo distant ou donnée privée n’est utilisé. Le test confirme son transport par pipe, son attachement au seul GET observé et l’absence dans la sortie du CLI; il ne prouve pas qu’un autre processus compromis sous le même UID ne puisse pas l’observer, ni qu’un serveur Forgejo live avec permissions réelles l’accepterait. Aucune règle d’autorisation serveur n’est créée. Pas de LLM/AgentOS, coût, modification de production ou publication.

Le runtime reste explicitement fixture-only dans les preuves actuelles; la comparaison contrôlée n’est pas exécutée. Les métadonnées Cargo ne sélectionnent aucune licence et ne déterminent pas les droits de redistribution. La mission reste incomplète.
