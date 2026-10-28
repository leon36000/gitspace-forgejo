# CONTINUATION-092 — refuser un timestamp `updated` mal formé au readback `workflow_run`

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dépôt fortement modifié.

## Scénario et résultat

Le vrai receiver accepte HTTP 202 pour une notification HMAC cohérente `workflow_run_success`, dépôt configuré, run 7001/index 41, avec timestamp et SHA signés valides. La fixture HTTP loopback répond au readback avec IDs, dépôt, statut, commit et URL attendus, mais `updated` vaut `not-a-timestamp`. Le vrai CLI `forgejo-reconcile-local`, lancé dans Bubblewrap, fait un GET au seul chemin du dépôt configuré puis retourne un JSON `inconclusive/invalid_current_state`, `retryable=false`, `details.field=updated`. Aucun `canonical_state` n’est produit, aucun marqueur privé ne fuite, le GET ne porte pas Authorization et le journal accepté reste inchangé octet pour octet.

Test : `actual_reconcile_cli_process_refuses_workflow_run_malformed_api_updated`. Commande :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_workflow_run_malformed_api_updated
```

Le driver compile hors ligne et lance le binaire de test dans Bubblewrap avec namespaces réseau/PID isolés, environnement effacé et sources read-only. Le test focalisé passe 1/1, 0 échec, 104 filtrés. Voir `process-e2e.log`.

## Vérifications

- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 34 exécutables, 597 réussis, 0 échec, 97 ignorés.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent.
- `python3 -m unittest scripts.test_cargo_license_state -v` : 9/9.
- `python3 scripts/verify_cargo_license_state.py` retourne `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`, `status=verified`.
- Outils et hashes des sources capturés dans `tool-versions.txt` et `source-sha256.txt`; les sorties de commande figurent dans les journaux du dossier.

## Portée et limites

Aucun code runtime ou dépendance n’a changé; le test ajoute uniquement une preuve processus supplémentaire pour le validateur de timestamp courant. L’API est une fixture loopback, pas Forgejo live ni un runner Actions. `n=1`; cela ne prouve ni la validité complète des dates acceptées, ni une fiabilité générale, ni un gain. Aucun LLM/AgentOS, coût, service externe, donnée privée, production ou publication.

Le manifeste Cargo technique reste résolu sans choix de licence et la publication est désactivée; les droits de redistribution restent non évalués. GitSpace conserve un statut de licence inconnu. La mission globale demeure incomplète, notamment la comparaison contrôlée Forgejo+agent+CI conventionnels et l’accès autorisé à un runtime agent réel.
