# Résultat sans descripteur de credential hôte

Commande exécutée depuis le worktree isolé :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_keeps_fork_pull_request_inconclusive_without_host_credential
```

Le runner a recompilé `webhook_receiver_process_e2e` hors ligne et avec Cargo `--locked`, puis a exécuté uniquement ce test ignoré dans Bubblewrap avec `--unshare-net --unshare-pid`. Le résultat observé par l’outil de commande : code de sortie 0; `1 passed; 0 failed; 0 ignored; 0 measured; 73 filtered out`.

La livraison HMAC et l’inbox passent par le receiver réel; le vrai CLI est lancé sans `--host-credential-fd`. L’API loopback représente un dépôt privé et renvoie 401 sans Authorization. Le test exige exactement un GET vers `/api/v1/repos/forge-admin/agent-forge-core/pulls/47`, aucun header Authorization, le verdict `inconclusive`, `upstream_unauthorized`, aucune donnée `canonical_state` et l’inbox byte-à-byte inchangée.

Ce contrat laisse le CLI effectuer un appel anonyme; il ne bloque pas localement avant l’appel. La preuve est une fixture, pas une API Forgejo vivante; aucun token réel, LLM, AgentOS, fusion, coût ou service externe n’est impliqué. Ce résultat est une note issue de la sortie observée, pas le stdout brut complet du runner.
