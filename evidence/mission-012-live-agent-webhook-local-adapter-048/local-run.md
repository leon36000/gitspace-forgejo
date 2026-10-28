# Mission 012 composée 048 — tentative de conflit même fichier

Date : 25 octobre 2026. Dépôt produit au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; le worktree reste fortement modifié.

Commande exécutée avec Forgejo local et `local-test-adapter` :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/mission012-same-file-conflict-048 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir evidence/mission-012-live-agent-webhook-local-adapter-048 \
  --execution-id live-mcp-006 \
  --allocation-id mission012-same-file-conflict-048 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter \
  --inject-concurrent-code-conflict
```

La scène Forgejo et l’autorité Rust ont atteint le refus attendu : deux branches ont des versions incompatibles de `src/lib.rs`; `git merge-tree --write-tree --name-only -z refs/heads/main refs/heads/agent/mission-012-live-agent-webhook-local-adapter-048` retourne 1 et nomme uniquement `src/lib.rs`. Le vérificateur indépendant passe pour le candidat. L’autorité retourne `invalid_review_base_binding`, sans autoriser ni tenter la fusion; la PR candidate reste ouverte/non fusionnée, tandis que la PR concurrente est la seule fusionnée. Forgejo 17.0.0 avait le SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

Le composeur finit néanmoins avec `FAIL-CLOSED: concurrent-change refusal readback does not prove that the target PR stayed unmerged` et code de sortie 1 : `validate_stale_base_readback` exigeait que le reçu agrégé conserve `concurrent_base_advance`, mais `validate_stale_base_refusal` ne copiait pas cet objet dans son résumé. Cette tentative n’est donc pas déclarée comme parcours composé réussi, même si les résultats Forgejo et du vérificateur sont présents. Le correctif transmet le readback concurrent et les deux champs qui le lient à la source; la relance complète 049 passe avec la même condition de conflit. Les preuves de 048 sont conservées.

L’adaptateur n’a invoqué aucun LLM; les `0.20 USD` sont le plafond de configuration du harnais, pas une dépense. Aucun service distant ou credential réel n’a été utilisé. La tentative démontre un défaut du validateur de reçu et son diagnostic, pas une fusion, une mesure de productivité ni la sécurité contre un écrivain non coopératif.
