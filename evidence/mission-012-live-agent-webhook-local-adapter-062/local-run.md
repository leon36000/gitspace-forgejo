# MISSION-012-COMPOSE-062 — reprise après SIGKILL jusqu’à la fusion Forgejo

Date : 25 octobre 2026. HEAD du dépôt produit examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le worktree est fortement modifié; ce rapport décrit l’exécution du runner sur son snapshot source, pas un commit propre.

## Périmètre et commande

Le parcours a été relancé contre une instance Forgejo locale temporaire, version `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le flag `--test-host-restart` demande au runner d’utiliser un host debug construit avec l’injection de crash; il est accepté uniquement avec `local-test-adapter`.

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/mission-012-forgejo-restart-062 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/mission-012-live-agent-webhook-local-adapter-062 \
  --execution-id live-mcp-062 \
  --allocation-id mission012-restart-062 \
  --agent-adapter local-test-adapter \
  --test-host-restart
```

Versions observées : Python 3.12.3, Rust/Cargo 1.95.0, Git 2.43.0, Linux aarch64. Les étapes et reçus complets sont `composed-run.json` et `composed-stage-summaries.json`; l’archive de snapshot de 83 fichiers a le SHA-256 `f2c5dff95cb7d67b96567bf677aeaafbf114afcfc7de41b4dfc6686f49bf1c28`.

## Résultat observé

L’événement Forgejo privé est lié à l’exigence `REQ-001`, SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`. Dans le projet de tâche `mission012-clamp`, la vérification de base échoue avec le retour `-2` alors que l’acceptation attend `10`; l’adaptateur produit le changement de `src/lib.rs`, puis les deux tests d’acceptation passent. La candidate modifie seulement `src/lib.rs`, son SHA-256 étant `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`.

Le point de crash `after_effect` termine le premier host après l’effet fichier et avant le résultat durable. Le premier appel sort avec le code 1, sans dépassement de sortie ni expiration du runner. Le journal relu contient un intent non résolu et zéro résultat. Après 10 250 ms d’attente d’expiration du bail, le nouveau host reprend la même opération/mission au fence 2. Le contexte de reprise est `unverified`; le client rejoue l’input MCP exact. Un résultat final est inscrit. Le reçu composé lie 5 événements du fence 1 et 8 du fence 2, soit 13 reçus.

Le parcours crée la PR #2 contre la base `5e621a3d593eb908b8ac4e3fe9e6d2d5f8bc1828`. Les essais de fusion avant revue et par le reviewer non autorisé reçoivent HTTP 405. `verifier-bot` fournit un verdict indépendant `pass`; l’autorité locale `forgejo-reconcile-local-rust` fusionne ensuite avec HTTP 200. Le readback confirme `main` sur `51860e96e265be51194e505429914af0e6512c9b`, PR/issue fermées et fusionnées, et 3 notifications réconciliées. Les empreintes des entrées d’arbre fusionné et candidat concordent (`5acd4a07d65b213d48afbf510a23dd1cce043a3faa61212a4e4d61af48792e74`). Le statut final est `merged_in_private_sandbox`.

Après le merge, j’ai exécuté `CARGO_TARGET_DIR=/tmp/mission-012-postmerge-062-target-20261025 cargo test --offline --locked --test acceptance` dans le snapshot candidat persisté. `src/lib.rs` correspond au SHA déclaré pour la candidate et les tests d’acceptation ont l’empreinte `5d4727542ea32f61aa4500754556fcfd1f39b59b4131ede257cb736fab231dc2`; résultat : 2 réussis, 0 échec. Le lien entre snapshot, candidate et arbre fusionné est structuré dans [la preuve post-merge](postmerge-062-verification.json), sortie brute [ici](postmerge-062-cargo-test.log). Le test a été lancé après la fusion mais sur le snapshot candidat existant, pas sur un nouveau checkout du commit de merge.

## Vérifications de cette reprise

- `python3 -m py_compile` sur les six runners/adaptateurs Python et les trois tests touchés : code retour 0.
- `python3 -m unittest test_forgejo_live_webhook_mission_admission test_mission_012_live_agent_pr_e2e test_mission_012_live_mcp_agent_e2e` : 62 tests réussis.
- `python3 scripts/verify_cargo_license_state.py` : état vérifié, `license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits de redistribution `not_assessed`.
- `python3 -m unittest discover -s scripts -p test_cargo_license_state.py` : 7 tests réussis.
- L’E2E 062 lui-même a terminé avec succès; ses reçus, résultats Forgejo et manifeste d’admission sont conservés dans ce dossier et `../mission-012-live-agent-webhook-local-adapter-062-admission/`.

## Limites

L’agent était `local-test-adapter`, déterministe : aucun LLM n’a été invoqué, aucun nombre de tokens n’est fourni et le coût `0.0` du reçu signifie qu’il n’y a pas eu d’appel modèle. L’effectif est une seule tâche; durée observée du segment agent : 12 636 ms. Cela ne mesure ni la productivité globale ni une différence face à Forgejo + agent + CI conventionnels.

Cette composition injecte le crash après l’effet; l’injection avant effet est vérifiée dans les tests isolés de reprise MCP, pas dans cet E2E Forgejo. La séparation par worktree ne sépare pas les privilèges des processus de même UID. Le verdict et les hashes établissent une provenance/relecture, pas la correction générale du code. La tâche est un petit projet Rust local de clamp numérique, pas un échantillon représentatif. Aucun service de production, publication ou modèle payant n’a été utilisé. Les refus composés encore manquants et le comparatif restent ouverts.
