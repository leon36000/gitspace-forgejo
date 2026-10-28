# CONTINUATION-119 — transmettre au MCP host le reçu de l’admission webhook

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, staging, publication ni changement de production.

## Changement et propriété

Le composeur MISSION-012 lançait jusque-là `mission-012-mcp-host` en mode `direct_local`, même quand le stage précédent avait admis une livraison Forgejo. Le nouveau module `scripts/mission_webhook_handoff.py` vérifie le fichier d’admission hôte, le reçu source de la livraison et l’inbox dans le répertoire `.runtime` privé. Il refuse les chemins déviés, fichiers non réguliers, liens, permissions, propriétaires ou tailles inattendus, incohérences d’allocation/scope/exigence/livraison et changements pendant une lecture.

Après validation indépendante de la sortie et des reçus du stage d’admission, `execute_stages` transmet uniquement les deux chemins exacts inbox/source au runner MCP. Celui-ci revalide le handoff, configure le host avec `admission_source_mode=webhook_delivery`, puis vérifie que le reçu source et l’inbox gardent les mêmes empreintes jusqu’à la fin de l’exécution. Le host Rust existant revalide le reçu contre le journal webhook avant de servir MCP. Les résumés composés lient l’ID de livraison et les hashes observés sans publier les chemins privés.

## Parcours local exécuté

Commande exécutée depuis la racine du worktree, dans un namespace réseau Bubblewrap distinct du namespace hôte (`net:[4026531833]`) :

```text
/usr/bin/bwrap --bind / / --dev-bind /dev /dev --proc /proc --unshare-net --die-with-parent -- /usr/bin/python3 scripts/run_mission_012_live_composed_e2e.py --state-dir /tmp/mission-012-admission-host-handoff-005 --forgejo-binary /tmp/forgejo-17.0.0 --evidence-dir <worktree>/evidence/mission-012-live-agent-webhook-host-handoff-005 --execution-id live-mcp-126 --allocation-id mission012-host-handoff-005 --budget-cap-usd 0.20 --expected-host-network-namespace net:[4026531833] --agent-adapter local-test-adapter --sandbox-test-adapter
```

Le code retour est 0. La livraison native Forgejo `issues/opened` `d7fc77cd-8907-4025-b0c5-e1af4300a020` a produit une admission pour l’exigence SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`. Le MCP host a rapporté le mode `webhook_delivery` avec le même ID; le hash du reçu source est `63a69aaed80b315a7d734399c9cb9a3acadcbd7a1ade93e8df20e80ca652bd8a`, et celui de l’inbox `16defc2bcdbea806c5b23af7cce5d645df8ca76901f7193f623c406571b0e3dc` avant et après le travail de l’adaptateur.

Le `local-test-adapter` a exécuté les outils MCP, observé le test rouge (`1` réussi, `1` échec : valeur `-2` au lieu de `10`), corrigé uniquement `src/lib.rs`, puis obtenu `2/2` tests d’acceptation. La PR locale #2 a été vérifiée indépendamment et fusionnée dans le bac Forgejo. Le commit candidat `c3092188be271b697cd083746dfc3836c54b4287` et le commit de fusion `44b1142e05d52d55aecde3d83106b6b09c136641` sont reliés aux reçus et au readback de `main`; l’acceptation sur checkout du commit fusionné donne `2 passed; 0 failed`. Les trois notifications de PR sont réconciliées. Voir `composed-run.json`, `composed-stage-summaries.json`, `independent-verifier.json`, `forgejo-result.json` et `postmerge-verification.json` dans ce dossier.

## Vérifications locales du changement

- `python3 -m py_compile scripts/mission_webhook_handoff.py scripts/run_mission_012_live_composed_e2e.py scripts/run_mission_012_live_mcp_agent_e2e.py` — réussi.
- `python3 -m unittest tests.test_mission_webhook_handoff tests.test_mission_012_live_agent_pr_e2e tests.test_mission_012_live_mcp_agent_e2e tests.test_composition_source_snapshot` — 70 tests réussis.
- Les tests dédiés vérifient le chemin attendu, la liaison de livraison, le chemin source hors runtime et le refus d’un reçu symlinké. Le parcours composé vérifie ensuite le handoff réel jusqu’au host MCP puis jusqu’au gate de fusion.
- `SHA256SUMS` du parcours composé valide les 16 artefacts initiaux. Les empreintes des scripts et tests effectivement capturés sont dans `composition-source-snapshot.zip`.

Deux exécutions de développement ont été conservées sans les compter comme réussite : l’une s’est arrêtée après l’admission parce qu’un champ de contrôle interne était inclus dans la comparaison des reçus; la suivante a terminé les trois stages puis a échoué lors d’une nouvelle comparaison du même reçu. Le handoff de contrôle a été sorti des données persistées de l’admission; la relance 005, dont tous les artefacts vérifiés sont ci-dessus, retourne 0. Les états `/tmp/mission-012-admission-host-handoff-003` et `-004` n’ont pas été supprimés.

## Portée et limites

Forgejo observé : version `17.0.0+gitea-1.22.0`, binaire local SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit source de construction de ce binaire n’est pas établi par cette exécution. L’audit séparé du code officiel v17.0.0 reste au commit `0d791421f85ca13a63b665053d81ea765fbbc571`.

Le seul agent utilisé est `local-test-adapter`: aucun LLM/AgentOS n’a été invoqué, aucun token de modèle n’a été consommé et `reported_cost_usd=0.0` ne mesure pas un coût de modèle. `n=1`; aucun gain comparatif n’est établi. Le processus MCP valide maintenant l’admission issue du webhook au démarrage, mais le même enfant MCP n’est pas gardé vivant jusqu’au gate de fusion dans cette composition; cette propriété plus stricte reste ouverte. Le cloisonnement testé n’établit pas l’exclusion générale des processus privilégiés ou du même UID.

Les droits GitSpace et la licence du produit restent non résolus; aucun code de ces sources n’a été copié. Les métadonnées Cargo restent sans licence sélectionnée et avec publication désactivée. La mission reste incomplète.
