# WEBHOOK-INVALID-CURRENT-STATE-PROCESS-001 — HTTP 200 ne suffit pas à établir un état courant

Date : 25 octobre 2026. Dépôt produit : HEAD de référence `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; worktree fortement modifié et préservé.

## Lacune visée

Le contrat unitaire refusait déjà les dates Forgejo invalides, mais le runner processus n’établissait pas encore qu’une réponse API HTTP 200 avec `updated_at` impossible reste inconclusive après ingestion du webhook, persistance dans l’inbox, puis réconciliation par le vrai CLI.

## Parcours et verdict observés

Le test `actual_reconcile_cli_process_refuses_success_http_with_invalid_timestamp` fait recevoir au vrai binaire webhook un événement signé de fixture pour l’issue 41, puis demande au vrai `forgejo-reconcile-local` de lire le journal durable. Une fixture HTTP loopback renvoie HTTP 200 avec `state="closed"`, l’URL attendue et `updated_at="2026-02-30T15:30:00Z"`.

Le processus CLI échoue avec un résultat JSON structuré `schema_version=2`, `verdict="inconclusive"`, `error.code="invalid_current_state"`, `retryable=false`, et une seule lecture amont. Aucun `canonical_state` n’est émis. La fixture observe exactement un `GET /api/v1/repos/forge-admin/agent-forge-core/issues/41`; la requête ne contient pas de header Authorization. Les octets du journal avant et après la réconciliation sont identiques. Le test ne traite donc pas le code HTTP 200 comme preuve de validité du contenu.

Le scénario dédié s’exécute avec `scripts/run_webhook_reconcile_cli_e2e.py` sous Bubblewrap (`--unshare-net`, processus et namespace de session isolés); il compile en mode hors ligne dans un workspace borné. Il n’utilise qu’une fixture locale, aucun accès réseau extérieur ni donnée privée.

## Commandes et résultats

```text
python3 -B scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_success_http_with_invalid_timestamp
cargo test --offline --locked --test webhook_reconcile_contract invalid_or_unrepresentable_forgejo_updated_at_is_rejected -- --exact --nocapture
rustfmt --edition 2024 --check tests/webhook_receiver_process_e2e.rs
python3 -B -m py_compile scripts/run_webhook_reconcile_cli_e2e.py
git diff --check -- docs/checkpoint.md docs/implementation-plan.md evidence/SHA256SUMS scripts/run_webhook_reconcile_cli_e2e.py tests/webhook_receiver_process_e2e.rs
```

Résultats frais : scénario processus `1 passed`, `0 failed`, `30 filtered out`; contrat unitaire `1 passed`, `0 failed`, `22 filtered out`. Rustfmt, compilation syntaxique Python, diff-check et scan des espaces finaux passent aussi. Les sorties des tests sont conservées dans `process-e2e-final.log` et `unit-contract.log`; versions et hashes sources dans les fichiers voisins. Le manifeste local vérifie 11 artefacts (`SHA256SUMS`), et `local-manifest-check.txt` en conserve la sortie 11/11.

Aucun code exécutable de production n’a changé dans cette tranche; le changement ajoute un test processus et son nom au runner. `Cargo.toml` et la décision de licence n’ont pas été modifiés.

## Limites

- `n=1`, fixture HTTP déterministe; ce résultat n’observe pas une instance Forgejo réelle.
- La suite `--all-targets` n’a pas été relancée dans cette tranche; le contrat ciblé et un seul scénario processus sont vérifiés.
- Aucun agent LLM, AgentOS, Forge Code, RAG ou mémoire n’est appelé; aucun token, coût de modèle ou gain comparatif n’est mesuré.
- Ce cas couvre une date de calendrier impossible, pas toutes les formes d’état malformé, les changements concurrents ou une preuve périmée.

La mission reste incomplète. Le prochain lot peut poursuivre le balayage LFS expérimental et reprenable dans le scratch Forgejo v17, tout en sélectionnant d’autres refus produit composés sans rejouer les parcours déjà prouvés.
