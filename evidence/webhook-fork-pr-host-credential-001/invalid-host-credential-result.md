# Résultat du refus de credential sur une PR de fork

Commande exécutée depuis le worktree isolé :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_fork_pull_request_with_invalid_host_credential
```

Le runner a recompilé `webhook_receiver_process_e2e` hors ligne et avec Cargo `--locked`, puis a exécuté uniquement ce test ignoré dans Bubblewrap avec `--unshare-net --unshare-pid`. Le résultat observé par l’outil de commande : code de sortie 0; `1 passed; 0 failed; 0 ignored; 0 measured; 72 filtered out`.

Le test traverse le receiver et le CLI réels du projet, avec une livraison HMAC synthétique, un FD contenant un faux token et une API loopback qui attend un token différent. Il vérifie le refus 401, exactement un GET vers `/api/v1/repos/forge-admin/agent-forge-core/pulls/46`, le verdict `inconclusive`, l’erreur terminale `upstream_unauthorized`, l’absence de `canonical_state`, l’absence du faux token dans stdout/stderr et les octets inchangés de l’inbox. Le résultat est une note d’exécution issue de la sortie observée; ce fichier n’est pas le stdout brut complet du runner.

Limites : pas d’instance Forgejo, de credential réel, de LLM ou d’AgentOS. Cette composition éprouve un token invalide; elle ne prouve pas le comportement en absence de descripteur de credential. Aucun code runtime n’a été modifié.
