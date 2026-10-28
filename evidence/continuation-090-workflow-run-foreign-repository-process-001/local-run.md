# CONTINUATION-090 — refuser un dépôt étranger au readback `workflow_run`

Date : 26 octobre 2026. Worktree produit : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié.

## Scénario et résultat

Le receiver Rust accepte HTTP 202 un événement HMAC `workflow_run_success` cohérent, attaché au dépôt configuré et au run 7001/index 41. La fixture API renvoie l’ID et l’index attendus, des champs d’état valides et l’URL canonique, mais le champ repository désigne explicitement `other-owner/other-repo`. Le CLI réel `forgejo-reconcile-local`, lancé par le driver sous Bubblewrap, fait un unique GET vers le chemin API du dépôt configuré puis retourne `inconclusive/resource_mismatch`. Aucun état canonique n’est projeté, aucun détail du dépôt étranger ou marqueur privé ne fuit dans stdout/stderr et le journal durable reste octet pour octet inchangé. La requête ne contient pas de header Authorization.

Test : `actual_reconcile_cli_process_refuses_workflow_run_foreign_api_repository`. Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_workflow_run_foreign_api_repository`. Le log archivé comprend la compilation hors ligne puis l’exécution isolée avec namespaces réseau/PID, environnement effacé et sources read-only.

## Vérifications

- Test processus focalisé : 1 réussi, 0 échec, 102 filtrés.
- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 34 groupes, 597 réussis, 0 échec, 95 ignorés.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent.
- `python3 -m unittest scripts.test_cargo_license_state -v` : 9/9.
- Le vérificateur Cargo courant retourne `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`, `status=verified`. Aucun choix de licence n’est fait; publication reste désactivée.
- Versions et hashes : `tool-versions.txt`, `source-sha256.txt`, `cargo-license-verify.json`.

## Portée et limites

Le receiver et le CLI sont de vrais programmes Rust, mais le readback est simulé par une fixture HTTP loopback, pas une instance Forgejo live ni un runner Actions. Cette preuve ferme ce seul cas de dépôt différent dans la réponse `workflow_run`; elle n’est pas un benchmark, une preuve de source de workflow ou une autorisation de fusion. `workflow_run` demeure consultatif. Aucun changement runtime, dépendance ou lockfile n’a été effectué. Aucun LLM/AgentOS, service externe, secret utilisateur, coût, production ou publication n’intervient.

La mission reste incomplète : parcours LLM/AgentOS autorisé, inventaire exhaustif, suite cumulative complète de refus et comparaison appariée avec Forgejo+agent+CI conventionnels restent à démontrer. Prochaine tranche : cibler un refus `workflow_run` encore absent du processus (l’état `commit_sha` nul), puis continuer l’audit cumulatif sans répéter les cas déjà prouvés.
