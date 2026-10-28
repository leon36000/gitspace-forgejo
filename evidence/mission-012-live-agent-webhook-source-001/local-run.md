# Parcours composé avec sources courantes — run positif 001

Date : 24 octobre 2026. But : réexécuter le parcours local complet après avoir retrouvé une dérive de sources dans les anciens runs CPU-055/056, et conserver les octets exacts exécutés.

## Commande et environnement

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/forgejo-mission012-composed-source-drift-001 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir evidence/mission-012-live-agent-webhook-source-001 \
  --execution-id live-mcp-091 \
  --allocation-id mission012-source-drift-001 \
  --budget-cap-usd 0.01 \
  --agent-adapter local-test-adapter
```

Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, loopback `127.0.0.1:3132`. Python 3.12.3, Rust/Cargo 1.95.0. HEAD source `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree reste sale et non commité. La provenance reliant ce binaire Forgejo au commit source officiel v17.0.0 n’est pas établie.

## Résultat observé

- `status=merged_in_private_sandbox`, issue fermée, PR #2 fusionnée par `forgejo-reconcile-local-rust`, readback réussi. Base `edd537e63ad7713341d261d9dfd656d8398dea77`, candidat `135e99d7240e73d603600716843845738b646e5d`, merge `cffca415d979fc48a561280b24bcc3d6171fc53d`.
- Le candidat ne change que `src/lib.rs`. L’acceptance de base échoue sur `values_outside_the_range_clamp_to_the_nearest_boundary` (`-2` au lieu de `10`); le candidat et la revérification prémerge passent. Les deux tentatives prématurées sont refusées par Forgejo en HTTP 405; le vérificateur indépendant et la revue liée passent.
- Le ledger lie cinq réservations résolues à la mission; le coût d’exécution déclaré est `0.0` et `model_invoked_during_run=false`.

## Provenance et limites

La map des 15 sources de composition concorde avec le workspace courant. Les 51 entrées `source_sha256` de l’admission concordent également. Les 55 fichiers distincts sont conservés dans [`source-snapshot`](source-snapshot/), avec leur manifeste propre (`56` entrées avec README). Le code source exécuté par l’orchestration et l’hôte d’admission est archivé. Le binaire Forgejo est seulement identifié par SHA; il n’est ni inclus ni relié par preuve de build au commit v17 audité.

Le résultat reste un seul parcours avec `local-test-adapter`, qui effectue une édition déterministe du fixture; ce n’est pas une démonstration de raisonnement LLM ni une mesure comparative. La demande et les critères sont préparés dans le fixture. Les services partagent l’UID hôte; les protections locales ne prouvent pas une isolation multi-tenant. Le cap `0.01 USD` est une valeur d’exécution enregistrée, pas une garantie de facturation; aucun modèle n’a été appelé.

Les artefacts et leurs SHA sont dans ce dossier et son `SHA256SUMS`; l’admission, son build, son webhook et son cleanup sont dans [l’évidence d’admission](../mission-012-live-agent-webhook-source-001-admission/).
