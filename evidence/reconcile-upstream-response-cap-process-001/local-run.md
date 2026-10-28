# RECONCILE-UPSTREAM-RESPONSE-CAP-PROCESS-001 — réponse chunked trop volumineuse

Date : 27 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours vérifié

Le test ignoré `actual_reconcile_cli_process_refuses_unbounded_chunked_current_api_response` démarre le receiver Rust, accepte une livraison d’issue signée HMAC avec un secret synthétique temporaire, puis lance le vrai `forgejo-reconcile-local`. Dans Bubblewrap, les espaces réseau et PID sont séparés (`--unshare-net --unshare-pid`); le seul serveur amont est une fixture sur `127.0.0.1:3130`.

La fixture répond `200` en `Transfer-Encoding: chunked`, sans `Content-Length`, et propose un corps non-JSON de 2 MiB alors que le budget annoncé par l’adaptateur est de 1 MiB. Le CLI termine avec le code 1 et une réponse JSON `inconclusive/upstream_invalid_response`, `retryable=false`, une seule lecture, sans `canonical_state`. Sa sortie reste sous 16 KiB et ne contient pas un segment du corps. La requête observée est le GET attendu de l’issue, sans `Authorization`; l’inbox reste identique octet pour octet.

Commande :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_unbounded_chunked_current_api_response
```

Le test processus passe 1/1; son reçu est `process-green.log`. Le premier lancement d’une variante `Content-Length` supérieur à la limite avait également passé; `content-length-first-run.log` est conservé comme essai exploratoire et ne remplace pas le test final chunked. La suite `cargo test --offline --locked --all-targets` passe 658 tests, 0 échec, 123 ignorés, sur 36 cibles; le test processus est ignoré par Cargo et lancé séparément sous Bubblewrap. `cargo fmt --all -- --check`, `git diff --check` et l’analyse AST du runner Python passent.

## Licence et portée

`cargo metadata --offline --locked --no-deps --format-version 1` et `scripts/verify_cargo_license_state.py` confirment encore `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed` et `NOASSERTION`; aucune licence produit n’est choisie.

La preuve couvre un seul corps HTTP chunked déterministe et un seul processus local. Elle ne mesure pas les octets effectivement consommés avant annulation, n’exerce pas Forgejo vivant, toutes les formes d’en-têtes/transfert, une charge prolongée, un LLM ou AgentOS, ni la production. Elle confirme le refus applicatif et l’absence de mutation du journal dans ce scénario; elle ne constitue pas une certification de la frontière réseau ou de la sécurité générale de curl.

Les sources et versions d’outils, les sorties de tests, l’état de licence et les empreintes append-only sont joints à ce dossier.
