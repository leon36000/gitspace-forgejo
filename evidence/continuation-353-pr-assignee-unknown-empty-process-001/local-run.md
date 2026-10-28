# CONTINUATION-353 — sémantique `null` / liste vide pour les assignees PR

Date : 2026-10-28. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Cette tranche ajoute un test de processus et son entrée au runner allowlisté; la logique produit existante n’est pas modifiée.

## Scénario

Le receiver signe et journalise deux événements `pull_request/synchronized` pour des PR distinctes. Chacun transporte une identité d’assignee périmée différente. Le vrai `forgejo-reconcile-local` traite les deux IDs dans un batch et reçoit deux réponses API loopback : `assignees: null` pour la PR 49 et `assignees: []` pour la PR 50.

Résultat : le premier état canonique conserve `assignees: null` (inconnu), tandis que le second retourne un ensemble vide avec `source=current_forgejo_resource_api`, `freshness=current_api_read` et `trust=untrusted_forgejo_identity`. Les deux lignes d’événement restent dans le même ordre; exactement deux GET ciblent les deux PR configurées; aucune requête ne porte Authorization, aucune identité périmée ne paraît dans stdout et le journal reste identique octet pour octet.

Commande :

```text
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_distinguishes_unknown_and_empty_pull_request_assignees
```

Le runner compile le binaire requis et exécute le test Rust `#[ignore]` sous Bubblewrap `--unshare-net`. [`isolated-test.log`](isolated-test.log) : 1 réussi, 0 échec, 172 filtrés. [`checks.log`](checks.log) : rustfmt, diff et AST Python passent.

## Portée et limites

Les cas `null` inconnu et `[]` vide connu existaient au niveau contrat, et un processus issues les couvrait déjà; cette tranche exerce les mêmes sémantiques à travers le chemin PR du vrai CLI et un batch multi-ressource. Le endpoint HTTP est une fixture locale, pas Forgejo live. Le test n’ajoute ni nouvelle logique, ni merge, ni agent/LLM; il ne prouve pas le comportement du serveur en production ou un gain. La suite `--all-targets` n’a pas été répétée.
