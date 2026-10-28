# WEBHOOK-BATCH-PREFLIGHT-001 — prévalidation de lot avant accès aux secrets

## Résultat

`src/bin/forgejo-reconcile-local.rs::run_reconciliation` refuse maintenant un lot de plus de 100 identifiants dès l’entrée de la fonction, avant de rejouer le journal d’entrée et avant de lire le descripteur de credential hôte. La réponse reste un refus structuré : `schema_version=3`, verdict `inconclusive`, code `batch_too_large`, `retryable=false`, maximum 100, action indiquant de scinder le lot, et zéro lecture amont.

Le test `oversized_batch_is_rejected_before_host_credential_read` fabrique une livraison signée locale, envoie 101 occurrences de son identifiant et fournit un FD invalide. Il vérifie le refus structuré et `upstream_read_attempts=0`. Le secret utilisé par la fixture est synthétique.

Empreinte SHA-256 du fichier source vérifié : `1a0a1435253fc517d2ddecaf3052c97e94213de8e4530c1b7d43c636cb8775c4`.

## Vérifications

- `cargo fmt --all -- --check` : réussite.
- `git diff --check` : réussite.
- `cargo test --offline --locked --all-targets` : 429 tests réussis, 0 échec, 23 ignorés. Commande lancée sous Bubblewrap avec l’espace réseau désactivé, cache Cargo en lecture seule et cible de compilation temporaire. Le test de prévalidation est inclus (binaire réconciliateur : 101/101; contrat `webhook_reconcile_contract` : 21/21).
- `python3 scripts/run_webhook_reconcile_cli_e2e.py` : 13/13 scénarios de processus réussis. Le runner compile hors namespace en mode Cargo `--offline`; chaque scénario s’exécute dans son propre namespace réseau Bubblewrap. Ces scénarios utilisent des fixtures HTTP privées et ne lancent pas Forgejo.
- `python3 scripts/test_cargo_license_state.py` : 5/5 tests réussis; `python3 scripts/verify_cargo_license_state.py` confirme `license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits de redistribution `not_assessed`.
- `cargo metadata --offline --no-deps --format-version 1` confirme le même état de licence.

Le premier appel `cargo test --offline --locked --bin forgejo-reconcile-local` a donné 100 tests réussis et un échec du test de processus `receiver_process_journal_reconciles_through_local_http_adapter`, car ce périmètre ne construisait pas le binaire compagnon `forgejo-webhook-local` requis par le test. Le test indique d’utiliser `--all-targets`; cette commande complète, qui construit le compagnon, passe ensuite. Ce premier résultat est conservé dans `bin-tests.log` et n’est pas présenté comme une réussite de suite.

Versions : voir `tool-versions.txt`. Sorties complètes : `all-targets.log`, `bin-tests.log`, `process-e2e.log`, `license-tests.log`, `license-state.json` et `cargo-metadata.json`.

## Limites

Cette preuve teste l’adaptateur Rust et des fixtures locales; elle ne démontre ni appel à un LLM, ni service Forgejo réel dans ce lot, ni gain comparatif. Le runner de processus prouve d’autres chemins de réconciliation; il ne comporte pas encore de cas E2E propre aux 101 identifiants. Les 23 tests ignorés de `--all-targets` ne sont pas comptés comme exécutés par cette commande.
