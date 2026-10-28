# Mission 012 composée 049 — refus de fusion sur conflit de code même fichier

Date : 25 octobre 2026. Dépôt produit au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; worktree fortement modifié. Forgejo local v17.0.0, Python 3.12.3 et Git 2.43.0. Le binaire Forgejo utilisé est `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; il n’a pas été reconstruit reproductiblement depuis les sources.

Commande exécutée :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/mission012-same-file-conflict-049 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir evidence/mission-012-live-agent-webhook-local-adapter-049 \
  --execution-id live-mcp-006 \
  --allocation-id mission012-same-file-conflict-049 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter \
  --inject-concurrent-code-conflict
```

Le composeur retourne code 0 avec `refused_after_concurrent_code_conflict_before_merge`. Il traverse l’admission webhook, les exigences hashées, la mission, le contexte et les outils MCP de l’adaptateur, la modification de `src/lib.rs`, la PR Forgejo #2, la vérification indépendante et la décision de l’autorité Rust. Le commit de base est `351810417bc306ed0e4f70e59061c327f5313ba0`, le commit candidat `44ebe52a800a40c856653d41f84a1ba8ea72ff69`, l’exigence SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431` et la politique SHA-256 `e23907b5bd6b2c0f342c88ddcfa9f9a9be98065adcd24f9f7299dc796b5739eb`.

Les tests de base échouent comme prévu sur la borne supérieure (`left: -2`, `right: 10`; processus Cargo code 101). Le candidat fixe la borne; les deux tests d’acceptation passent. Les trois vérifications indépendantes enregistrées sont `base-red` (101 attendu), `candidate-green` (0) et `premerge-green` (0). La source candidate a le SHA-256 `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`.

Après la vérification du candidat, une PR concurrente approuvée modifie la même ligne de `src/lib.rs` en une expression différente (`maximum - 1`) et est fusionnée seule. La sonde Git `merge-tree --write-tree --name-only -z refs/heads/main refs/heads/agent/mission-012-live-agent-webhook-local-adapter-049` retourne 1 avec le chemin de conflit unique `src/lib.rs`; le hash de sa sortie est `23ec4bea05e2981cee8a252e802e47b2541ffb6b87c680ca327e619d1ec53530`. Le contenu brut de la sonde n’est pas conservé.

L’autorité refuse ensuite la PR candidate avec `invalid_review_base_binding`, `merge_authorized=false` et `merge_attempted=false`. La PR candidate demeure `open`, `merged=false`; la PR concurrente #3 est fusionnée en HTTP 200 et `main` pointe sur `93cd0826db30207a112be704f5427bfbbbfcf3f5`. Le SHA-256 de la source concurrente sur `main` est `15435b094b716996fa9928d0f20ca4362f9b4fad219bc1dd3c922d0c27ade37c`, distinct de celui du candidat. La relecture finale confirme ces états et la liaison du reçu composé. La capture de provenance couvre 83 fichiers et 2 595 671 octets; l’archive est hashée dans `composed-run.json`.

La première tentative 048 a trouvé un défaut de composition : le reçu agrégé omettait la PR concurrente et le readback final échouait fermé. Les champs ont été ajoutés au résumé validé, puis l’exécution complète 049 a réussi. Cette correction du harnais est conservée et détaillée dans [le rapport 048](../mission-012-live-agent-webhook-local-adapter-048/local-run.md).

Il s’agit d’un test isolé avec `local-test-adapter`; aucun LLM n’a été invoqué, aucun token modèle consommé et le coût déclaré de l’adaptateur est nul. `0.20 USD` est le plafond déclaré, non une dépense. L’instance Forgejo et les identifiants synthétiques sont locaux; aucun code Forgejo n’est copié au produit. Ce cas prouve le refus de fusion d’une PR candidate lorsque le même fichier a divergé de façon incompatible dans une PR concurrente. Il n’exerce pas des écritures réellement simultanées, n’exclut pas un writer qui ignore le verrou coopératif et ne mesure pas d’avantage face à Forgejo + agent + CI conventionnels. Le parcours positif de fusion est celui de 046 et n’est pas rejoué ici.
