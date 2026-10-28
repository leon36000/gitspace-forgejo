# MISSION-012 — parcours webhook signé → agent → PR (23 octobre 2026)

## Résultat observé

Le script composé a terminé avec le code 0 dans un Forgejo privé de test. Il a utilisé, dans le même état privé, le receiver webhook Rust, le CLI Rust d’admission, l’hôte MCP, Claude Code Opus 5.5/high, le vérificateur Cargo isolé et l’API Forgejo locale. La commande n’a modifié que le dépôt synthétique de la fixture; elle n’a pas écrit dans le dépôt source de cette forge.

La base admise est `b8691e537725c8a92435262d338b6f58019a6b28`, l’issue est #1 et l’exigence `MISSION-012-CLAMP-001` est liée par le SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`. Le DTO hôte produit par l’admission a le SHA-256 `9cac854ad0fe71c5f800a517210b06eb51255051359108d30b3738e611950b30`.

Le receiver a refusé la mauvaise signature (HTTP 401, aucun octet journalisé), accepté l’événement signé (202, séquence 1), puis reconnu le même identifiant livré à nouveau (202, une seule entrée). L’admission anonyme a été refusée avant claim (`mission_source_forbidden`). Le rejeu de l’admission a rendu `already_claimed`, conservé la même empreinte de DTO et n’a pas ajouté d’événement.

Claude a reçu le contexte lié à l’issue et au commit admis par les quatre outils MCP `mission.context`, `workspace.read_text`, `workspace.write_text` et `cargo.test`. Les huit reçus contiennent six appels d’outil et une finalisation `candidate_ready`; ils lient l’allocation, la base, l’exigence, le workspace et les chemins. Le premier test a échoué sur la borne supérieure (`clamp_to_range(15, -2, 10)` rendait `-2` au lieu de `10`); l’agent a changé uniquement `src/lib.rs`, puis le test MCP a passé 2/2. Le SHA-256 de cette source candidate est `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`.

Forgejo a créé la PR #2. Le vérificateur distinct a récupéré le commit candidat `0de80101ec54454bc762823a239fd79f8a21de6d`, confirmé que seul `src/lib.rs` différait, obtenu la défaillance attendue sur la base (1 passé, 1 échoué), puis 2 tests passés sur le candidat dans Bubblewrap avec réseau, home et credentials absents et Cargo verrouillé hors ligne. Il a revérifié le candidat immédiatement avant l’appel de fusion. Forgejo a refusé la fusion avant revue (HTTP 405) et celle du réviseur non autorisé à fusionner (HTTP 405). L’approbation de `verifier-bot` était liée au SHA candidat. L’owner de la fixture a ensuite fusionné (HTTP 200); le commit de merge et la tête `main` relue sont `9ea0c2e4d8c2a0deec59e7046bd64b26445fe7e3`. La PR est fermée/fusionnée, l’issue est fermée et le contenu relu correspond à l’empreinte vérifiée. Le reçu MCP `b734768775718b0c917da6904de6bc8a1f22c6fb485e0e423c1b38a3a5275af8` est le même dans la preuve de fusion.

Le résumé Claude indique 16,449 ms pour le tour agent, 10 tokens d’entrée et 900 tokens de sortie, et un coût de **0,0745424 USD**. Le temps pipeline, mesuré entre la création du journal orchestration et l’écriture du résultat composé, est d’environ **32,6 s**; ce n’est pas le temps complet depuis la préparation de l’instance. Les compteurs token et coût rapportés par Claude ne sont pas indépendamment attestés, et le sens exact du compteur d’entrée n’a pas été validé. Une tentative composée précédente avait exécuté le même travail agent mais échoué dans le driver PR avant fusion; son coût CLI rapporté est 0,0894398 USD. Les deux exécutions LLM observées dans cette tranche totalisent donc 0,1639822 USD d’après la CLI; ce n’est pas un relevé de facturation. Chaque exécution a conservé le plafond configuré de 0,20 USD.

## Reproduction

Depuis le worktree courant :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.tools/forgejo-17.0.0-linux-arm64 \
  --state-dir /tmp/mission012-webhook-composed-run-005 \
  --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/mission-012-live-agent-webhook-compose-005 \
  --execution-id live-mcp-006 \
  --allocation-id mission012-webhook-live-005 \
  --budget-cap-usd 0.20
```

Versions observées : Forgejo `17.0.0+gitea-1.22.0`, SHA-256 binaire `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; Claude Code `2.1.309`; Python `3.12.3`; Rust/Cargo `1.95.0`; Bubblewrap `0.9.0`. Le HEAD du dépôt source est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; il n’a pas bougé pendant ce travail. Le worktree contient d’autres modifications préexistantes et celles de cette tranche; aucun commit n’a été créé. Les empreintes des six fichiers de code/test touchés figurent dans le manifeste.

Le run composé a utilisé `run_mission_012_live_composed_e2e.py` SHA-256 `02e0ab826a5b55cd94127ffa4fc10ad32b133a167884cf75b3d74252568bf79f`. Après ce succès, la version de travail a ajouté les compteurs token au résumé expurgé et durci l’arrêt d’enfants descendants après timeout; le test de non-survie a d’abord échoué puis passé. Ces retouches ont passé les 18 tests ciblés ci-dessous, mais aucun troisième appel LLM n’a été lancé. Le manifeste identifie le code présent maintenant; il ne faut pas le confondre avec le hash exact du script du run.

Tests ciblés après les corrections du driver :

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest \
  tests.test_mission_012_live_composed_e2e \
  tests.test_mission_012_live_admission_e2e \
  tests.test_mission_012_live_agent_pr_e2e -v
```

Résultat : 18 tests, 0 échec, journalisé dans [les tests ciblés](mission-012-live-agent-webhook-compose-005-targeted-tests.log). Ils couvrent notamment refus de l’admission anonyme, doublon signé, échec d’étape qui arrête la suite, liaison DTO/reçus/candidat, dépassement de budget, sortie enfant plafonnée, arrêt d’un descendant après expiration, candidat qui imprime un faux succès, binding de revue, compteurs résumés sans chemin privé et parents de merge exacts. `git diff --check` passe. Les fichiers de preuve détaillée sont dans [le répertoire du run](mission-012-live-agent-webhook-compose-005/) : résumé composé, read-back Forgejo, verdict du vérificateur, sorties rouge/vert, refus de fusion et revérification pré-fusion.

## Couverture et limites

- C’est un parcours de processus local intégré avec une instance et un dépôt Forgejo de fixture; ce n’est ni un déploiement, ni une observation d’usage réel avec de vrais utilisateurs ou projets.
- Le reviewer `verifier-bot` et les deux identités Forgejo sont des comptes synthétiques. Aucun examen humain n’est démontré; l’approbation de test n’est pas une preuve de correction.
- HMAC, hashes et reçus attestent intégrité/provenance locale, pas correction. L’état privé est protégé par les modes de fichiers et le worktree, mais ces mécanismes n’isolent pas un processus hostile du même UID. Le driver PR détient l’autorité owner de la fixture; il ne constitue pas une autorité de fusion de production.
- Le driver relit base, tête, protection, approbation et reteste juste avant le merge. L’API Forgejo testée ne compare pas atomiquement la base attendue à la fusion; un writer non coopératif conserve une fenêtre TOCTOU. Ce run n’a pas injecté un changement concurrent de base/tête au moment de la fusion.
- Le doublon testé est la répétition identique d’une seule livraison. Les tests séparés refusent l’échec d’étape et les bindings discordants, mais cette exécution n’a pas simulé une interruption puis reprise au milieu d’une fusion, des livraisons désordonnées distinctes ou deux agents en conflit sémantique.
- Le vérificateur lance ici la suite d’acceptation épinglée sur les octets du commit Forgejo exact. Le dépôt fixture ne versionne pas `policy/active.json` ni la policy v4; le SHA de policy est fixé dans le fixture hôte. `mission-012-verify-local` ne valide donc pas ce dépôt fixture.
- Aucune intégration AgentOS, Forge Code, RAG ou mémoire n’a été observée. Aucun MCP générique ni accès shell n’a été donné à Claude. Aucun statut publié par l’exécutant ne remplace le verdict indépendant.
- Effectif de la démonstration avec LLM : `n=1` parcours complet réussi. Il n’y a ni groupe de référence Forgejo + agent + CI conventionnelle, ni variation statistique, ni mesure d’interventions humaines, ni comparaison contrôlée de temps/coût. Aucun gain général n’est établi.
- Les états privés des essais `/tmp/mission012-webhook-composed-run-002` à `-005` et leurs journaux restent conservés; les trois premiers essais ont échoué avant fusion et n’ont pas été supprimés. Le serveur Forgejo du run réussi est arrêté et le port loopback 3132 est libre.

## Empreintes du code de cette tranche

Les SHA-256 exacts des scripts et tests composant ce parcours sont consignés dans le manifeste voisin, avec ceux des éléments de preuve. La revue indépendante Opus/Ralph demandée séparément s’est arrêtée sans livrer de résultat exploitable; aucun avis de cette revue n’est présenté ici comme validé.
