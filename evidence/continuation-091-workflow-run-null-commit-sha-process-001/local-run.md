# CONTINUATION-091 — refuser un `commit_sha` nul dans le readback `workflow_run`

Date : 26 octobre 2026. Worktree produit : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, état Git fortement modifié.

## Scénario et résultat

Le vrai receiver Rust accepte HTTP 202 pour un événement HMAC cohérent `workflow_run_success` lié au dépôt configuré, run 7001 et index 41; le SHA signé est valide. La fixture HTTP loopback répond au readback du run avec les IDs, dépôt, statut, date et URL canoniques, mais `commit_sha` vaut JSON `null`. Le vrai CLI `forgejo-reconcile-local`, lancé dans le bac à sable Bubblewrap par le driver, fait un GET au seul chemin du dépôt configuré puis termine en échec avec un JSON structuré `inconclusive/invalid_current_state`, `retryable=false`, `details.field=commit_sha`. Aucun `canonical_state` n’est émis, aucun marqueur privé du webhook/API ne fuite, la requête n’envoie pas Authorization et le journal HMAC accepté reste identique octet pour octet.

Le test processus est `actual_reconcile_cli_process_refuses_workflow_run_null_api_commit_sha`. Commande finale après `cargo fmt` :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_workflow_run_null_api_commit_sha
```

Le driver compile hors ligne puis exécute le test dans Bubblewrap avec namespaces réseau et PID isolés, environnement effacé et sources read-only. `process-e2e-final.log` est la preuve de référence; `process-e2e.log` conserve le premier run focalisé.

## Vérifications

- Test processus focalisé : 1 réussi, 0 échec, 103 filtrés.
- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 34 exécutables, 597 réussis, 0 échec, 96 ignorés.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent.
- `python3 -m unittest scripts.test_cargo_license_state -v` : 9/9.
- Le vérificateur de métadonnées Cargo retourne `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`, `status=verified`.
- Versions des outils et hashes SHA-256 des sources exécutées : `tool-versions.txt`, `source-sha256.txt`; preuves de commande dans les journaux adjacents.

## Portée et limites

Le test exerce des processus Rust réels et le refus par le code de réconciliation courant; l’API est une fixture locale et non une instance Forgejo, les runners Actions ne sont pas lancés. `workflow_run` demeure consultatif et ce cas ne crée aucune autorisation de fusion. `n=1`; ce résultat ne mesure ni la fiabilité générale ni un gain produit. Aucun changement de code runtime, dépendance, lockfile ou configuration de production. Aucun LLM/AgentOS, coût, service externe, secret utilisateur, publication ou changement de licence.

La mission reste incomplète : la comparaison appariée Forgejo+agent+CI conventionnels n’est pas exécutée; AgentOS n’est pas constaté comme runtime utilisable ici; les droits GitSpace/redistribution restent non évalués et Cargo n’a pas de licence choisie. La prochaine tranche locale couvre le refus processus d’un timestamp `workflow_run.updated` mal formé, sans répéter les autres readbacks déjà prouvés.
