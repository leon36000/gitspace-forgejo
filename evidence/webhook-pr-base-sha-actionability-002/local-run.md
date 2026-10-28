# WEBHOOK-PR-BASE-SHA-ACTIONABILITY-002 — action opérateur vérifiée

Date : 2026-10-25. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout reste modifié et contient des sources non suivies; les empreintes du rapport identifient la source de travail.

## Résultat

Le scénario processus de réconciliation PR déjà présent vérifie maintenant aussi le texte exact de `error.action`, en plus du refus d’un `base.sha` mal formé. La réponse doit être `inconclusive`, `invalid_current_state`, `retryable=false`, `details.field=base.sha`, ressource PR #25, sans `canonical_state`, sans en-tête `Authorization` et sans changement du journal signé. L’action attendue est : « inspect the named field in the Forgejo API snapshot and retry only after the source is corrected ». Cela évite qu’un agent ou un opérateur ne relance indéfiniment la même entrée sans corriger la source.

Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reports_invalid_pull_request_base_sha_field` — 1/1, code 0. Le runner compile les exécutables réels du workspace puis exécute le scénario sous Bubblewrap 0.9.0 avec `--unshare-net`; le serveur API est une fixture HTTP loopback privée. Sortie et code : `invalid-pr-base-sha-actionable.log` et `.exit`.

Vérifications fraîches : `cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, `python3 scripts/test_cargo_license_state.py` (7/7), `python3 scripts/verify_cargo_license_state.py` (`verified`) — tous code 0 après correction du rapport de plan. Le premier `git diff --check` a signalé une ligne blanche en fin de `docs/implementation-plan.md` (code 2); elle a été retirée et le contrôle a été relancé avec succès. Les deux sorties sont archivées. Les autres sorties/codes sont conservés dans ce dossier; versions dans `tool-versions.txt`.

## Portée et limites

L’implémentation de `error.action` existait déjà; la tranche améliore le contrat testé et la preuve d’actionnabilité, sans modifier la logique de production. La réponse API est synthétique : aucune instance Forgejo réelle, fusion, AgentOS ou LLM n’a été utilisée. Aucun benchmark comparatif ou résultat d’usage n’est établi.

Cargo continue d’indiquer `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected` et `redistribution_rights=not_assessed`. Aucune licence produit n’est choisie et ce garde ne tranche pas les droits des dépendances.
