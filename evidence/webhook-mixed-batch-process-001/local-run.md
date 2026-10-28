# WEBHOOK-MIXED-BATCH-PROCESS-001 — batch mixte à travers receiver et CLI

## Parcours exécuté

`actual_reconcile_cli_process_mixed_batch_limits_credential_to_configured_repository` poste deux événements HMAC synthétiques au receiver Rust : une issue du dépôt configuré et une autre issue hors périmètre. Le receiver répond `202` aux deux et persiste les deux dans son journal. Le vrai processus `forgejo-reconcile-local` reçoit les IDs et un token synthétique via FD pipe.

Le test tourne dans Bubblewrap `--unshare-net`; un serveur loopback privé à `127.0.0.1:3130` sert la lecture issue 41 et accepte seulement l’Authorization synthétique. Readback : le CLI effectue un GET, retourne `current_state_read` pour la ligne autorisée et `upstream_forbidden` pour l’autre, verdict batch `inconclusive`, `upstream_read_attempts=1` et code d’échec attendu. Le test confirme qu’aucun chemin externe n’est requêté, que le credential est consommé pour l’événement admissible, et qu’il n’apparaît ni en stdout ni en stderr. Résultat isolé : 1/1.

Commande reproductible :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_mixed_batch_limits_credential_to_configured_repository
```

Le test frère, entièrement refusé avec FD non consommé, passe lui aussi 1/1 dans [WEBHOOK-DENIED-BATCH-PROCESS-001](../webhook-denied-batch-process-001/local-run.md). Le runner peut cibler un scénario; sa matrice complète n’a pas été rejouée.

## Contrôles et limites

`cargo fmt --all -- --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` et `git diff --check` passent. Le readback de licence frais est conservé dans le dossier frère : `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected`, `redistribution_rights=not_assessed`; `publish=false` demeure un garde local. Aucune licence n’est choisie.

Receiver et binaire réels du projet, mais test-only : le GET atteint une fixture loopback dans un namespace isolé, pas Forgejo. Aucun credential réel, LLM, AgentOS, gateway, PR, fusion, accès externe ou coût. Le scénario est ignoré par Cargo par défaut et doit passer par le runner sous Bubblewrap. Ce test ne démontre pas la compatibilité complète avec une version live de Forgejo ou la sûreté d’un credential de production.

Logs, versions et hashes exacts sont dans ce dossier; manifeste local `SHA256SUMS`.
