# WEBHOOK-UPSTREAM-401-ACTIONABLE-001 — action sûre après 401 Forgejo

Date : 25 octobre 2026. Worktree produit `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, encore fortement modifié. Aucun commit propre n’est revendiqué.

## Changement

Le mapping `ApiReadError::Unauthorized` de la réconciliation indiquait d’inspecter la réponse et les permissions sans dire que l’authentification hôte pouvait manquer, être périmée ou être rejetée. Le préflight de fusion avait le même défaut d’action. Les deux actions indiquent désormais de vérifier le credential Forgejo limité au dépôt configuré, de confirmer qu’il est présent, courant et accepté, et de garder sa valeur hors des entrées de l’agent. La classification reste `upstream_unauthorized`, non retryable; le 401 n’est pas transformé en succès et aucun état canonique n’est émis.

Le nouveau test processus `actual_reconcile_cli_process_reports_actionable_upstream_401_without_leaking_credential` envoie deux webhooks signés du même issue #41 avec deux IDs de livraison, puis exécute le vrai CLI de réconciliation. Le fixture HTTP de loopback renvoie 401 et un corps contenant un marqueur synthétique. Le host passe un token synthétique via FD; le fixture observe un seul GET et l’en-tête Authorization attendu. Le CLI rend deux refus `inconclusive/upstream_unauthorized`, `retryable=false`, sans `canonical_state`; ni le token ni le corps HTTP n’apparaissent dans stdout/stderr. Le cache du réconciliateur évite une seconde lecture du même ressource. Un test unitaire vérifie séparément l’action du préflight de fusion.

Le premier run TDD s’est arrêté sur l’assertion qui exigeait le mot `credential` dans l’action. Cet échec est décrit dans [l’extrait](tdd-red-excerpt.txt); le run vert final après formatage et lié aux hashes de sources consignés est dans [process-e2e-final.log](process-e2e-final.log). L’exécution verte précédente reste aussi archivée dans `process-e2e.log`.

## Vérifications

Outils : Rust/Cargo `1.95.0`, Python `3.12.3` (voir [versions](tool-versions.txt)).

- `PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reports_actionable_upstream_401_without_leaking_credential` — 1/1; le runner bâtit le scénario puis l’exécute sous Bubblewrap avec `--unshare-net`. Il s’agit du vrai receiver et du vrai CLI contre un serveur HTTP synthétique loopback, pas d’une instance Forgejo.
- `CARGO_TARGET_DIR=/tmp/agent-forge-webhook-046-target cargo test --offline --locked --all-targets -- --test-threads=1` — 462 réussis, 0 échec, 32 ignorés, 34 groupes; le code de sortie est 0. La sortie complète est dans [all-targets-final.log](all-targets-final.log).
- `CARGO_TARGET_DIR=/tmp/agent-forge-webhook-046-target cargo test --offline --locked --bin forgejo-reconcile-local merge_preflight_unauthorized_error_identifies_host_credential_action -- --test-threads=1` — 1/1; voir [cargo-401-unit.log](cargo-401-unit.log). Ce test est également dans la suite `--all-targets` ci-dessus.
- `PYTHONDONTWRITEBYTECODE=1 python3 scripts/verify_cargo_license_state.py` — statut `verified`, package `agent-forge-core`, `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected`, `redistribution_rights=not_assessed`; readback : [JSON](cargo-license-state.json). Ses 7 tests refusant une sélection ou une publication passent (journal [cargo-license-tests.log](cargo-license-tests.log)). Aucun choix de licence ni droit de redistribution n’est ajouté.
- `cargo fmt --all -- --check`, `git diff --check` et `PYTHONPYCACHEPREFIX=/tmp/agent-forge-webhook-python-cache python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent. Journaux de formatage et diff-check co-localisés.

Un essai `cargo test --bin forgejo-reconcile-local` sans les autres binaires a échoué dans un ancien test du dépôt, car celui-ci exige aussi `forgejo-webhook-local`; sa sortie est préservée dans `cargo-bin-tests.log`. La commande `--all-targets` requise a ensuite passé. Cet échec est un prérequis de construction du harnais, pas un défaut de ce changement.

## Licence et limites

Le dépôt conserve `publish = false` et des métadonnées de revue explicites. Le readback Cargo confirme l’absence de champ `license` et `license-file`; aucune licence n’est choisie. Le manifeste reste non publiable tant que les droits ne sont pas évalués.

Le scénario prouve la classification d’un 401 et la protection des sorties sur le receiver/CLI contre une fixture contrôlée; il ne démontre pas de token Forgejo réel, d’instance live, d’agent LLM, d’intégration AgentOS, ni de succès comparatif. Le succès de la suite locale ne prouve pas une amélioration des métriques de projet. Le code reste dans le worktree dirty et aucune donnée privée, aucun service externe, aucun déploiement ni coût nouveau n’a été utilisé.

Les empreintes des sources testées, de `Cargo.toml` et de `Cargo.lock` sont dans [source-sha256.txt](source-sha256.txt). Le manifeste local couvre les artefacts de cette tranche, sauf lui-même.
