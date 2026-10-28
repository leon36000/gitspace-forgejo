# Parcours composé 028 — refus de base périmée

Date : 24 octobre 2026. Forgejo local `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le run est privé et jetable; aucun déploiement ni publication.

## Reproduction

Depuis la racine du workspace :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary "$PWD/.tools/forgejo-17.0.0-linux-arm64" \
  --state-dir /tmp/mission012-composed-adapter-028 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-028" \
  --execution-id live-mcp-028 \
  --allocation-id mission012-local-adapter-028 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter \
  --inject-concurrent-base-advance
```

Le pipeline a reçu une vraie livraison Forgejo `issues/opened`, créé une admission versionnée, fourni le contexte à l’adaptateur MCP local, observé l’acceptance rouge sur la base, modifié `src/lib.rs`, puis obtenu le verdict indépendant `pass` sur le candidat `0912e4b8a446ee62061b8ae1695d2e060fb357dd` (SHA-256 source `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`). La tâche reste strictement limitée à ce fichier.

Le fixture a ensuite créé une PR concurrente README-only, l’a fait approuver par `verifier-bot` et l’a fusionnée par le compte owner local. `main` est passé de `84b5cc35df07ddae6afd7ea4ab44aa2b878d03f2` à `f3ea97abb076517b3d2fa45191b31021d93169af`. L’autorité de fusion Rust a relu la PR et rejeté le candidat avec `invalid_review_base_binding`; `merge_authorized=false` et `merge_attempted=false`. Le readback Forgejo montre la PR cible #2 ouverte et non fusionnée, l’issue #1 ouverte et le hash de `src/lib.rs` sur `main` toujours égal à la version cassée. Seule la PR concurrente #3 a avancé `main`.

Les manifestes SHA-256 de ce run et de son admission passent. Les détails liés sont `composed-run.json`, `forgejo-result.json`, `rust-merge-authority-result.json`, `concurrent-base-advance.json`, `independent-verifier.json`, les journaux Cargo rouge/vert et `SHA256SUMS`.

## Portée

`local-test-adapter` est déterministe; aucun LLM, AgentOS, Forge Code, RAG ou mémoire n’a été invoqué. Le verdict de test indépendant s’applique au commit candidat exact sous Bubblewrap/offline/locked. Le lock Rust de génération est coopératif et ne sérialise pas un writer Forgejo externe; l’API de merge Forgejo n’offre pas de CAS atomique sur la base entre sa dernière lecture et le POST. Le scénario prouve un déplacement de base **avant** la décision, pas l’absence de toute course externe au POST. Les identités et processus partagent l’UID local. Un parcours n’établit aucun gain général de réussite, de temps, de coût ou de tokens.
