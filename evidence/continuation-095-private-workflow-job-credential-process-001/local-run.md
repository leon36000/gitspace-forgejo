# CONTINUATION-095 — readback privé d’un job Actions et de son run parent

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree fortement modifié.

## Scénario et résultat

Le vrai receiver accepte HTTP 202 pour un `workflow_job_failure` HMAC lié au dépôt et au run attendus. Le CLI reçoit le token **synthétique** depuis un pipe anonyme hôte via `--host-credential-fd`. Le fixture loopback séquence deux réponses : job 9001 courant (`success`), puis run parent 7001 courant (`failure`, SHA/date). Le processus émet exactement deux GET dans l’ordre : `/actions/jobs/9001`, puis `/actions/runs/7001`; chacun porte `Authorization: token <token>`. Aucun GET de logs ou write n’est émis. Le résultat expose le statut courant du job et le SHA/date du parent, mais pas le token ni les marqueurs de nom/étapes/runner/event payload. Le journal HMAC demeure identique octet pour octet.

Test : `actual_reconcile_cli_process_reads_private_workflow_job_and_parent_run_with_host_credential`. Commande :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_private_workflow_job_and_parent_run_with_host_credential
```

Le driver compile hors ligne et exécute sous Bubblewrap avec namespaces réseau/PID isolés, environnement effacé et sources read-only. Résultat focalisé : 1 passé, 0 échec, 107 filtrés.

## Vérifications

- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 34 exécutables, 597 réussis, 0 échec, 100 ignorés.
- `cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent.
- `python3 -m unittest scripts.test_cargo_license_state -v` : 9/9.
- `python3 scripts/verify_cargo_license_state.py` confirme `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`, `status=verified`.
- Outils et empreintes des sources sont dans `tool-versions.txt` et `source-sha256.txt`; sorties archivées dans les logs du dossier.

## Portée et limites

C’est un test processus du receiver/CLI et de l’adaptateur courant, avec API fixture locale. Il vérifie le transport du credential et l’ordre des deux lectures, mais pas l’autorisation d’un Forgejo live, un runner Actions, un token réel, la résistance à un processus compromis partageant le même UID, ni l’exécution d’un agent. Aucun code runtime, lockfile, dépendance ou configuration de production n’a changé; aucun coût ou service externe n’est utilisé.

Cargo reste techniquement sans licence choisie et non publiable; cela ne détermine pas les droits de redistribution. AgentOS/modèle n’est pas intégré et la comparaison contrôlée n’est pas exécutée; la mission globale demeure incomplète.
