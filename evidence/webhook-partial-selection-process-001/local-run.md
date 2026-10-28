# WEBHOOK-PARTIAL-SELECTION-PROCESS-001 — refuser un batch partiellement résolu

## Contrat exercé

Le processus receiver réel accepte un webhook HMAC synthétique visant le dépôt configuré et l’écrit dans son inbox. Le processus `forgejo-reconcile-local` est ensuite lancé avec cet ID présent, suivi d’un second ID absent, et un FD pipe contenant un credential synthétique.

Sous Bubblewrap `--unshare-net`, le CLI renvoie le code 1 et un JSON batch `inconclusive` portant `delivery_not_found` sur le premier ID absent, `requested_delivery_count=2` et `upstream_read_attempts=0`. Le processus parent relit ensuite l’intégralité du pipe : le CLI n’a pas tenté de lire le credential pourtant utile à la première livraison. Aucun résultat partiel ni requête API n’est produit. Le scénario ciblé passe 1/1.

Cette preuve couvre le cas que WEBHOOK-RECONCILE-UNKNOWN-DELIVERY-001 avait explicitement laissé hors du test : une ligne sélectionnable suivie d’une ligne absente. Aucun code de production n’a changé; le lot ajoute un test de processus ignoré par Cargo par défaut et l’inscrit dans le runner `--scenario`.

## Reproduction et contrôles

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_partial_selection_before_credential_read
```

Le runner compile hors ligne dans un `target` temporaire, puis exécute le seul test dans un namespace réseau isolé. `cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` et `scripts/verify_cargo_license_state.py` réussissent. Readback Cargo : `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected`, `redistribution_rights=not_assessed`.

Versions, hashes des sources, journal complet du runner et manifeste local `SHA256SUMS` sont conservés dans ce dossier.

## Limites

Le test utilise le receiver et le CLI du worktree avec une inbox temporaire et des fixtures synthétiques; il ne lance pas Forgejo, ne fait aucune PR/fusion et n’utilise ni LLM ni AgentOS. Il vérifie ce refus en isolation, pas le comportement après fusion ni une comparaison avec une CI conventionnelle.
