# CONTINUATION-120 — refuser la lecture du runtime privé depuis le sandbox agent

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun stage, commit, publication, appel modèle ou changement de production.

## Résultat observé

Une exécution d’admission fraîche a créé `/tmp/mission-012-private-runtime-sandbox-001` et `evidence/mission-012-private-runtime-sandbox-001`. Forgejo local v17.0.0 a émis une livraison native `issues/opened` `36d13213-46c3-449d-8abe-176e0a0befb4`; le receiver l’a reçue en HTTP 202, le CLI l’a liée à la mission et à l’exigence SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`. L’admission a été produite sur la base `27ebd0154fbb4ce732c8188fd8acff6fe63b0e3d`. Le manifeste de ces dix fichiers d’admission est vérifié par `sha256sum -c` (10/10); il est référencé sans copier l’inbox, les tokens ou le secret du hook.

Le stage suivant a lancé le binaire MCP Rust SHA-256 `015ea0a8cb339ba9370c4c88b805707ddefd74743606d96faf929ae051f3e92d` avec `local-test-adapter` derrière Bubblewrap 0.9.0. Avant de lancer l’enfant, le wrapper trusted a confirmé les types de fichiers, liens, propriétaire UID courant, permissions `0600`, répertoire `.runtime` `0700` et noms attendus. Depuis le namespace du processus sandbox, l’adaptateur a ensuite tenté d’ouvrir ces six chemins exacts :

- `host_admission` — DTO `host-admission.json`;
- `admission_source` — reçu lié à la livraison;
- `webhook_inbox` — inbox JSONL;
- `mission_journal` — journal d’exécution;
- `forgejo_read_token` et `invalid_forgejo_read_token`.

Les six tentatives ont échoué par absence dans le namespace. Les fichiers et leurs contenus ne sont pas montés; le reçu ne contient que les étiquettes. Le résultat lie le même ID de livraison, le hash de source `1fa7772fca441b65bbbe674348f065d20a5ef84e5c6ebc21e69e82e7e27da4cd`, l’inbox `5169960021f685835de52c76e91168e095b475a7305a4fe2c4f6ed8ad894c3e6` et la candidate `src/lib.rs` SHA-256 `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`. Le fixture a réellement observé l’échec d’acceptation, corrigé le fichier via MCP et passé les tests protégés; l’agent ressort `candidate_ready_for_independent_verification`. Résumé et sondes : `agent-run-summary.json`, `sandbox-checks.json` et `admission-linkage.json`.

## Vérifications

- `python3 -m py_compile` des trois scripts et deux tests concernés : réussi.
- `python3 -m unittest -v tests.test_mission_012_sandboxed_test_adapter tests.test_mission_012_live_mcp_agent_e2e tests.test_mission_012_test_agent_adapter tests.test_forgejo_agent_egress_live_e2e tests.test_mission_webhook_handoff tests.test_composition_source_snapshot` : 43 réussis, 0 échec.
- Admission réelle + stage MCP relié, `local-test-adapter`, résultat code 0. `agent-run-summary.json` consigne `model_invoked_during_run=false`, tokens `null`; le `reported_cost_usd=0.0` du fixture ne mesure pas un coût modèle.
- `sha256sum -c evidence/mission-012-private-runtime-sandbox-001/SHA256SUMS` : 10/10.
- Versions et hash des scripts/binaries : `toolchain.txt`, `source-hashes.txt`; commandes : `commands.txt`.

## Limites et tentatives non comptées

Une première invocation avait repris l’état `mission-012-admission-host-handoff-005` après sa fusion antérieure. Le runner a refusé avant l’exécution de l’agent avec `trusted Forgejo Git head differs from the admitted base commit`; l’admission y était périmée. Cette invocation n’est pas comptée comme preuve positive. L’état temporaire n’a pas été supprimé.

Le run compté est `n=1`, n’exécute pas de LLM, AgentOS ou MCP de production et ne crée ni PR ni fusion. Il prouve la visibilité de ces chemins sous le montage Bubblewrap observé, dans cette configuration Linux. Les processus trusted partagent l’UID hôte; un agent exécuté hors de ce sandbox, un autre montage, une évasion, le noyau ou un acteur privilégié ne sont pas couverts. Aucun merge ou parcours complet n’est revendiqué par ce lot; la continuité du même enfant MCP de l’admission jusqu’au gate demeure ouverte.
