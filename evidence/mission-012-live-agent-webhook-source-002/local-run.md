# Parcours composé avec sources courantes — refus concurrent 002

Date : 24 octobre 2026. But : confirmer avec les sources courantes que le gate refuse la fusion si la branche de base avance après la vérification du candidat.

## Commande et environnement

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/forgejo-mission012-composed-source-drift-002 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir evidence/mission-012-live-agent-webhook-source-002 \
  --execution-id live-mcp-092 \
  --allocation-id mission012-source-drift-002 \
  --budget-cap-usd 0.01 \
  --agent-adapter local-test-adapter \
  --inject-concurrent-base-advance
```

Même binaire Forgejo v17 local, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree sale, Python 3.12.3 et Rust/Cargo 1.95.0 que le [run positif 001](../mission-012-live-agent-webhook-source-001/local-run.md). Les deux maps de sources sont identiques; le snapshot exact conservé pour les deux runs est [source-snapshot](../mission-012-live-agent-webhook-source-001/source-snapshot/). Le binaire Forgejo est identifié par SHA mais n’est pas inclus ni relié par preuve de build au commit v17 audité.

## Résultat observé

- Le candidat est vérifié (`verifier=pass`) après l’échec attendu de l’acceptance sur la base et sa correction. Une PR cible #2 est créée; le run ouvre aussi la PR concurrente #3 et avance `main` à `e465ffdbb06f1e1e926bcd6902d832f5ac3f1ad5`.
- L’autorité refuse ensuite avec `invalid_review_base_binding`; `merge_authorized=false`, `merge_attempted=false`, PR/issue cibles encore ouvertes. `status=refused_after_concurrent_base_advance_before_merge`.
- Les cinq réservations CPU sont résolues; `model_invoked_during_run=false`, `reported_cost_usd=0.0`.

## Limites

Run n=1 de refus contrôlé dans Forgejo privé local; ce test ne prouve pas une résistance à tout writer externe ni à toute course de production. L’adaptateur est déterministe et sans LLM. Les services partagent l’UID hôte; le binaire local Forgejo n’est pas relié par preuve de build au commit source audité. Résultat complet, tâches, vérification, readback et hashes : ce dossier et [l’évidence admission](../mission-012-live-agent-webhook-source-002-admission/).
