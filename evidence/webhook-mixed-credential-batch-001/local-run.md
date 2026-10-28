# WEBHOOK-MIXED-CREDENTIAL-BATCH-001 — credential uniquement pour les ressources autorisées

## Changement et vérification

Le test `mixed_batch_reads_host_credential_only_for_in_scope_rows` sélectionne une livraison HMAC valide pour `forge-admin/agent-forge-core` et une autre visant `other-owner/other-repository`. Le vrai `run_reconciliation` lit un credential synthétique depuis un pipe, vérifie l’état courant de la première ressource et conserve `upstream_forbidden` pour la seconde. Le batch rend `inconclusive`, compte exactement un GET, et la fixture loopback n’enregistre que le chemin du dépôt configuré avec l’en-tête synthétique.

Résultats sur le code après Rustfmt : `cargo test --offline --locked --bin forgejo-reconcile-local` donne 104 réussis, 0 échec, 0 ignoré; le filtre `host_credential` donne 11 réussis, 0 échec; le nouveau test ciblé donne 1/1. `cargo fmt --all -- --check` et `git diff --check` passent. Journaux détaillés dans ce dossier.

## Cargo sans choix de licence

`Cargo.toml` conserve `publish = false`, n’a ni clé `license` ni `license-file`. La garde fraîche `scripts/verify_cargo_license_state.py` retourne `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected`, `redistribution_rights=not_assessed`, `status=verified`. Aucun choix de licence, ajout de fichier ou changement de `Cargo.toml`/`Cargo.lock` n’a été fait dans ce lot. Cela maintient la publication désactivée et ne résout pas les droits de redistribution.

## Runtime agent et modèle

La recherche source constate `AgentRuntimeV1` dans `src/agent_runtime.rs:3225` et `FixtureRuntime` comme implémentation dans `tests/agent_runtime_contract.rs:46–50`; aucune implémentation provider de production n’apparaît dans `src/`/`tests/`, et `Cargo.toml`/`src/` ne déclarent pas de client AgentOS, Forge Code, Ollama, Anthropic, OpenAI ou LiteLLM. L’inventaire local mentionne un gateway et un dashboard, mais marque modèle, autorisation et coût comme non vérifiés; aucun appel n’a été fait. Aucun runtime parallèle ni intégration n’est ajouté; la fixture HTTP est explicitement un test.

## Limites

Le test utilise un journal temporaire, un token synthétique et une fixture HTTP loopback : aucun Forgejo live, credential réel, PR, fusion, agent LLM, service externe ou comparaison de valeur. La suite de tous les targets (431 réussis/23 ignorés) date d’avant l’ajout de ce test; ce lot a vérifié tout le binaire concerné et les filtres ciblés, sans refaire `--all-targets`. Le test ne prouve pas qu’un fournisseur de modèle est actuellement autorisé ou abordable.

Références et empreintes exactes : [preuve précédente des refus avant lecture](../forgejo-reconcile-credential-preflight-001/local-run.md), `runtime-interface-search.txt`, `source-sha256.txt` et `tool-versions.txt`.
