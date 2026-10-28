# WEBHOOK-RESOURCE-KIND-CACHE-045 — identité issue/PR dans un batch

Date : 25 octobre 2026. Worktree produit au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, toujours dirty et partagé; aucun commit propre n’est revendiqué. Source testée : `src/bin/forgejo-reconcile-local.rs`, SHA-256 `008fadc2d3267b6bc67d217d35ac17857facda2f5770a277c1e38e8b08548f4f`.

## Comportement vérifié

Le test `batch_cache_keeps_issue_and_pull_request_with_same_number_separate_over_http` traverse `LocalLoopbackForgejoApi` puis `reconcile_inbox_events`. Il donne au batch une issue 19 et une PR 19 du même dépôt. Le fixture loopback répond respectivement à `/issues/19` avec un état fermé et à `/pulls/19` avec un état ouvert, non fusionné, et des SHA de tête/base valides. Le serveur observe exactement les deux GET distincts. Les réponses ne se croisent pas dans le cache : l’issue garde `head_sha/base_sha/merged` absents et la PR garde ses champs propres.

Commande :

```sh
CARGO_TARGET_DIR=/tmp/agent-forge-webhook-045-target cargo test --offline --locked --bin forgejo-reconcile-local batch_cache_keeps_issue_and_pull_request_with_same_number_separate_over_http -- --nocapture
```

Résultat : 1 réussi, 0 échoué, 0 ignoré, 107 filtrés; Cargo `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, Rust `rustc 1.95.0 (59807616e 2026-04-14)`. `rustfmt --edition 2024 --check src/bin/forgejo-reconcile-local.rs` et `git diff --check` passent également. La sortie brute et le résultat JSON liant commande, source, scénarios et versions sont dans ce dossier; le manifeste local les couvre.

## Portée et limites

C’est un test de contrat du vrai adaptateur et du réconciliateur batch contre un serveur HTTP loopback synthétique; ce n’est pas une instance Forgejo, et aucune route de production n’a été exercée. Le code de comportement produit n’a pas changé : cette tranche ajoute une garde de régression pour une clé de cache typée par ressource Forgejo (un numéro d’issue et un numéro de PR peuvent coïncider). Aucun LLM/AgentOS n’a été utilisé. Cargo reste explicitement non licencié et non publiable : `license=null`, `license_file=null`, `publish=[]`, choix `unselected`, droits `not_assessed`.
