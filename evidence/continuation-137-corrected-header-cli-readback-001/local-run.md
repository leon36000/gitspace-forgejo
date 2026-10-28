# CONTINUATION-137 — readback CLI après correction d’un en-tête webhook

## Résultat

Le test `actual_reconcile_cli_process_reads_corrected_header_retry_from_receiver_journal` compose le vrai processus receiver avec le vrai binaire `forgejo-reconcile-local`.

1. Un corps d’issue valide et HMAC-signé, étiqueté `pull_request` dans l’en-tête non signé, reçoit HTTP 400 `event_payload_invalid`; l’inbox ne contient rien.
2. Le même identifiant, corps et signature, étiqueté correctement `issues`, reçoit HTTP 202, `duplicate=false`, séquence 1 et `reconcile_required=true`. Le journal contient exactement un record `forgejo.webhook.v1.issues`.
3. Le CLI lit cette livraison dans un processus séparé. Sous Bubblewrap `--unshare-net`, il effectue exactement un GET à `/api/v1/repos/forge-admin/agent-forge-core/issues/41`, n’envoie aucun en-tête `Authorization` et reçoit la réponse de la fixture loopback privée.
4. Le corps signé annonce `issue.state=closed`; la réponse API fixture annonce `state=open`. Le verdict CLI est `current_state_read`, la projection utilise `open`, et le journal reste identique octet pour octet après readback.

## Vérification

Commande :

```text
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_corrected_header_retry_from_receiver_journal
```

Le runner a copié seulement `Cargo.toml`, `Cargo.lock`, `src`, `tests` et `build.rs` dans un dossier temporaire, compilé hors ligne, puis exécuté le test `--ignored --exact` dans Bubblewrap avec réseau et PID isolés, sources en lecture seule, environnement vidé, cache Cargo local en lecture seule et budgets bornés. Résultat : 1 réussi, 0 échec, 0 ignoré, 114 filtrés. `cargo fmt --all -- --check`, l’analyse syntaxique Python du runner et `git diff --check` passent. L’exécution n’a touché qu’au test, à l’inscription du scénario dans le runner et à la documentation/evidence; aucun runtime n’a changé.

## Limites

L’API est une fixture loopback, non Forgejo; le payload et la clé HMAC sont synthétiques. Le parcours démontre le chaînage du receiver au CLI et un readback canonique contrôlé, pas un webhook émis par Forgejo live. Un seul événement et une seule ressource sont couverts; aucune mesure de valeur, LLM, AgentOS ou benchmark. Aucun secret privé ou service externe n’a été utilisé. Le projet reste incomplet.

## Révision, sources et intégrité

HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Voir `commands.txt`, `sandboxed-cli-e2e.log`, `rustfmt.log`, `python-parse.log`, `diff-check.log`, `tool-versions.txt`, `source-hashes.txt` et `manifest-prefix-before.json`. Les index locaux, worktree et projet sont append-only; le vérificateur confirme le préfixe existant et le binding du parent après append.
