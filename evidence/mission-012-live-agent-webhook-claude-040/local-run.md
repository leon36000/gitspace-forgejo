# MISSION-012 — parcours Forgejo local avec Claude Opus 5.5 (run 040)

Date : 24 octobre 2026. Code source examiné au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Instance et dépôt privés jetables sous `/tmp`; aucun changement de production, commit ou publication.

## Parcours observé

Le run **040** complète webhook signé → admission et exigence liée → contexte MCP → échec acceptance sur la base (`left: -2`, attendu `right: 10`) → correction par modèle → acceptance verte → vérification indépendante → PR #2 → revue → refus des merges avant revue et par le reviewer non autorisé (HTTP 405) → fusion par `forgejo-reconcile-local-rust` → readback d’issue fermée. Un seul fichier change, `src/lib.rs`. Base `1e981dcf9bd0e53aa52dc74c87c068c4c19fd2a0`, candidat `72f47607fbe7b407831f7520f2f5ffaa3054a3c1`, merge `86f92d38e6dcf27ff8147fc4f215959d5d948a1f`. Le hash du fichier candidat est `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`.

Le reçu MCP lie les appels observés de contexte, lecture/écriture de workspace et test Cargo. Le vérificateur relance les critères sur le candidat et retourne `pass`; le readback Forgejo lie la PR fusionnée au candidat. Les artefacts détaillés sont dans [composed-run.json](composed-run.json), [composed-stage-summaries.json](composed-stage-summaries.json), [l’acceptance rouge](base-red-cargo-test.log), [l’acceptance verte](candidate-green-cargo-test.log), [la vérification indépendante](independent-verifier.json) et le [manifeste](SHA256SUMS).

## Modèle et limites de la mesure

Le préflight `claude --version` juste avant le run a retourné `2.1.310 (Claude Code)` ([sortie](cli-version.txt)). La commande de l’intégrateur a configuré le modèle `claude-opus-5-5`, effort `high`, plafond demandé `0.20 USD`, 12 tours maximum et quatre outils MCP. Le choix d’adaptateur entraîne aussi les champs `model` et `effort` des reçus : ni ces champs ni le fournisseur n’attestent indépendamment quel modèle a effectivement servi la requête.

Le résumé CLI d’étape rapporte 0,0869206 USD, 10 tokens d’entrée, 928 de sortie et 14 378 ms. Les trois mesures tokens/durée sont présentes dans [composed-stage-summaries.json](composed-stage-summaries.json); leurs champs correspondants dans `composed-run.json` valent `null`. Le coût est reproduit dans les deux résumés. Ces valeurs ne sont pas une preuve indépendante de facturation ou d’usage fournisseur. Un ancien essai du dépôt avec plafond demandé 0,20 USD a rapporté 0,5258432 USD avec `error_max_budget_usd`; ce plafond n’est donc pas une borne financière fiable pour les prochaines exécutions.

Ce parcours prouve qu’un vrai LLM a consommé le contexte/outillage et produit un changement exécutable qui a passé une vérification distincte et la fusion locale. `n=1` ne mesure pas un gain général. Le run 039 sur la même tâche utilisait l’adaptateur déterministe; ce n’est pas une baseline appariée Forgejo+CI conventionnelle. Aucun comparatif de réussite, régressions, interventions, temps total ou coûts n’est établi. AgentOS, Forge Code, RAG et mémoire ne sont pas intégrés à cette tranche.

Commande reproductible (attention : le plafond CLI n’est pas une limite de facturation garantie) :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/mission012-composed-claude-040 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-claude-040" \
  --execution-id live-mcp-040 \
  --allocation-id mission012-claude-040 \
  --budget-cap-usd 0.20 \
  --agent-adapter claude
```

Le run 039 déterministe et le run 040 avec CLI configurée pour Opus sont conservés séparément; les états privés de test `/tmp/mission012-composed-claude-040` ne sont pas supprimés.
