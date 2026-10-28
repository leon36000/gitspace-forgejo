# Parcours composé 029 — fusion autorisée par l’autorité Rust

Date : 24 octobre 2026. Forgejo local `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Run privé et jetable, sans déploiement ni publication.

## Reproduction

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary "$PWD/.tools/forgejo-17.0.0-linux-arm64" \
  --state-dir /tmp/mission012-composed-adapter-029 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-029" \
  --execution-id live-mcp-029 \
  --allocation-id mission012-local-adapter-029 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter
```

Le parcours a lié la livraison webhook à l’admission, la génération, l’exigence et la base; l’adaptateur local a consommé le contexte MCP, observé l’acceptance rouge, corrigé `src/lib.rs`, puis l’acceptance indépendante sur le candidat `d5805073241a1734f12c709fed5a8e521888dc8e` a passé. La PR #2 a été revue par `verifier-bot`; les fusions avant revue et par le reviewer non autorisé ont reçu HTTP 405. `forgejo-reconcile-local` a autorisé et effectué la fusion. Le readback confirme le merge `4ccc3a8c3f0900dc34ba50cb9355f11ab725b5b8` sur `main`, ses parents attendus base/candidat, le tree testé et l’issue #1 fermée.

Les manifestes de la preuve composée et de l’admission passent. Voir `composed-run.json`, `forgejo-result.json`, `rust-merge-authority-result.json`, `independent-verifier.json` et `SHA256SUMS`.

## Portée

L’exécuteur est `local-test-adapter`; zéro appel modèle et zéro coût de modèle. Le vérificateur adapte la policy fixture versionnée depuis l’hôte, car ce dépôt minimal ne versionne pas `policy/active.json`. L’isolation du candidat est locale et les processus partagent l’UID; aucun essai ne démontre AgentOS, Forge Code, RAG ou mémoire. Le verrou hôte ne protège que contre les allocateurs coopératifs; Forgejo n’offre pas de CAS atomique sur la base pour un writer externe. C’est un succès de parcours, pas une mesure comparative ou un résultat de production.
