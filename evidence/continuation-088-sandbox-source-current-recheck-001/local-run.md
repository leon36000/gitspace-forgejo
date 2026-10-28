# CONTINUATION-088 — revalider la frontière sandbox sans répéter le merge live

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dépôt fortement modifié.

## Résultat

La demande de vérifier l’impossibilité pour le processus agent isolé d’atteindre les routes Forgejo ou de lire le credential du merge était déjà démontrée par deux runs préservés. Je n’ai pas relancé la fusion : [FORGEJO-AGENT-STDIO-MCP-MERGE-001](../forgejo-agent-stdio-mcp-merge-001/local-run.md) maintient le vrai child `--stdio-mcp` derrière Bubblewrap pendant un gate Forgejo vivant. Le readback assaini montre trois probes API/Web/`git-receive-pack` bloquées, namespaces réseau et PID séparés, FDs enfant `0,1,2`, credential `memfd` CLOEXEC non hérité, accès `/proc` refusé, lecture de base HTTP 200 et merge autorisé HTTP 200. Le post-merge verifier passe 9 tests. Le run emploie `local-test-adapter`, pas un LLM ou AgentOS. [FORGEJO-AGENT-EGRESS-STDIO-MCP-007](../forgejo-agent-egress-stdio-mcp-007/local-run.md) fournit aussi le run live des routes contre le listener Forgejo. Les manifestes SHA-256 conservés vérifient respectivement 27/27 et 22/22 artefacts.

J’ai vérifié ces deux manifestes, les sorties JSON et la provenance. Les onze fichiers produit capturés correspondent exactement aux onze hashes source du run de merge. Sept ont changé depuis ce run; les différences examinées sont listées dans `source-reconciliation.txt`. Le snapshot live demeure la preuve du comportement sur ses sources exactes, pas une preuve automatique au HEAD courant. Sur les sources courantes, le vrai processus Bubblewrap/MCP est retesté avec un serveur synthétique : `tests.test_mission_012_sandboxed_test_adapter` passe 7/7; les gardes du rapport live passent 9/9 via `tests.test_forgejo_agent_egress_live_e2e`. Le premier teste le processus et les refus réseau/credential actuels, tandis que l’accès gate/merge réel provient du run live archivé.

Les tests ne prétendent pas qu’une liste de routes côté client forme la frontière : la preuve réseau vient du namespace `--unshare-net` du processus et les tests ciblent un listener vivant; la séparation de credential vient de l’absence de FD hérité, de l’isolation PID et du probe `/proc`. Le test courant est contre une fixture. Le run Forgejo live antérieur est contre son snapshot source exact.

## Vérifications

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_mission_012_sandboxed_test_adapter -v` : 7 réussis.
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_forgejo_agent_egress_live_e2e -v` : 9 réussis.
- `sha256sum -c SHA256SUMS` dans les deux répertoires de preuve live : tous les artefacts passent.
- Sources exactes des 11 fichiers historiques et écarts présents au HEAD : `source-reconciliation.txt`; empreintes présentes : `current-source-sha256.txt`.
- Versions : `tool-versions.txt`.

## Limites et suite

Le fixture ne démarre pas Forgejo pendant ce recontrôle, et je n’ai pas rejoué le merge. Les routes API/Web/receive-pack testées sont les routes explicites de la sonde; `receive-pack` est sondé sans authentification. Le credential de fusion reste dans la zone hôte de confiance et les writers externes/concurrents du serveur ne sont pas exclus. Un UID partagé, une évasion noyau, une frontière multi-tenant et un modèle/AgentOS autorisé ne sont pas couverts. Aucun code runtime n’a été modifié, aucun LLM appelé, aucun coût ou service externe engagé, aucune production touchée.

Prochaine tranche : traiter une autre lacune de la matrice de refus qui ajoute une nouvelle preuve intégrée, sans relancer les scénarios d’egress ou de merge déjà enregistrés. Le benchmark apparié et l’intégration AgentOS restent ouverts; le projet est incomplet.
