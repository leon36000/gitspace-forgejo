# CONTINUATION-089 — refuser une URL étrangère au readback workflow_run

Date : 26 octobre 2026. Worktree produit : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié; aucun reset ni commit.

## Scénario et résultat

Le receiver Rust accepte HTTP 202 une notification HMAC `workflow_run_success` cohérente, liée au run 7001 et à l’index 41 dans le dépôt configuré. Le readback HTTP du vrai CLI `forgejo-reconcile-local` renvoie ensuite un run de même ID, index, dépôt, statut, commit et date, mais avec `html_url=https://attacker.invalid/...`. Le CLI, exécuté par le driver sous Bubblewrap, sort en échec structuré `inconclusive/resource_mismatch` après exactement un GET du run configuré. Il ne projette aucun `canonical_state`, ne divulgue ni l’URL étrangère ni les marqueurs privés du payload, et ne réécrit pas le journal durable. La requête ne comporte pas d’Authorization.

Test : `actual_reconcile_cli_process_refuses_workflow_run_foreign_api_html_url`. Driver : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_workflow_run_foreign_api_html_url`. Le journal du processus inclut la compilation hors ligne, la commande d’isolation (`--unshare-net`, `--unshare-pid`, environnement effacé, sources read-only) et le résultat 1/1.

## Vérifications

- Test processus focalisé : 1 réussi, 0 échec, 101 filtrés.
- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 34 groupes, 597 réussis, 0 échec, 94 ignorés.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent.
- Versions et empreintes des sources/lockfile : `tool-versions.txt` et `source-sha256.txt`.

## Portée

Ce scénario vérifie l’identité de lien renvoyée pour `workflow_run`; il complète le refus d’ID divergent de CONTINUATION-087. Le receiver et le CLI sont de vrais binaires de la suite, mais l’API est une fixture HTTP loopback, pas une instance Forgejo ou un runner Actions. `workflow_run` reste une observation consultative, jamais une autorisation de merge ou une preuve de test. Aucun code runtime, dépendance ou lockfile ne change. Aucun LLM, AgentOS, secret utilisateur, coût, service externe, production ou publication n’intervient.

Le résultat n’établit pas toutes les incohérences API possibles, ni un gain comparatif. Les parcours LLM/AgentOS autorisés, le benchmark apparié, le détecteur général de conflit sémantique et la couverture complète des writers Forgejo restent ouverts; la mission est incomplète.
