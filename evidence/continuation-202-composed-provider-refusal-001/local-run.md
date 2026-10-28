# CONTINUATION-202 — refus fournisseur avant préparation du parcours composé

Date : 2026-10-27. HEAD de base observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree déjà fortement modifié. Versions et empreintes du code/documents concernés : [`source-hashes.json`](source-hashes.json). Préfixes globaux observés avant ce lot : [`manifest-prefix-before.json`](manifest-prefix-before.json).

## Constat et correction

L’entrée agent MCP avait déjà une garde fail-closed en CONT-199 : `scripts/run_mission_012_live_mcp_agent_e2e.py::run` refuse Claude avant résolution de l’état; `build_agent_command` refuse avant la recherche du binaire. L’entrée composée `scripts/run_mission_012_live_composed_e2e.py::run` n’avait pas cette garde et lisait d’abord l’argument d’état, vérifiait le namespace réseau et capturait les sources. Son constructeur `build_stage_commands` traitait le budget avant qu’un refus parvienne au stage MCP. Cela créait un écart de contrôle précoce, même si l’enfant existant refusait ensuite l’adaptateur Claude.

Le runner composé et son constructeur rejettent maintenant `agent_adapter == "claude"` immédiatement avec : `Claude Code execution is disabled: provider egress and filesystem isolation are not attested`. Le premier point se trouve au début de `run()` (vers la ligne 2184); le second au début de `build_stage_commands()` (vers la ligne 151). Le refus du constructeur précède validation du budget, des chemins et production des commandes de stages. Le refus de `run()` précède lecture du namespace, capture de sources, écriture d’état et lancement de stage. Le test emploie des mocks qui échouent si inspection réseau ou capture source est appelée.

## Résultats reproductibles

Avant correction, les deux nouveaux tests échouaient : `run()` accédait à `state_dir` sans refuser le fournisseur, et `build_stage_commands()` rendait une erreur de budget à la place du refus d’isolation. Voir [`red.log`](red.log). Après correction :

- `PYTHONPATH=tests python3 -m unittest -v test_mission_012_live_composed_e2e` : 14 tests réussis, 0 échec; reçu [`green.log`](green.log).
- Les deux tests focalisés sur les nouveaux refus : 2/2; reçu [`provider-refusal-tests.log`](provider-refusal-tests.log).
- Les deux tests existants du runner MCP (`test_claude_command_refuses_unisolated_provider_before_cli_lookup`, `test_model_run_refuses_before_resolving_state_or_credentials`) : 2/2; reçu [`mcp-refusal-tests-corrected.log`](mcp-refusal-tests-corrected.log).
- Une première commande de sélection MCP a utilisé un nom de classe inexistant et a échoué au chargement avant les tests; cet essai est préservé dans [`mcp-refusal-tests.log`](mcp-refusal-tests.log), puis relancé avec le nom réel `AgentCommandTests`.
- AST Python des deux fichiers modifiés : 2 fichiers passés; `git diff --check` : réussi, sans sortie. Reçus [`syntax-check.log`](syntax-check.log), [`diff-check.log`](diff-check.log).

Environnement : Python 3.12.3, systemd 255.4, Linux aarch64. Il s’agit de tests unitaires du lanceur; aucun scénario Forgejo, local-test-adapter, LLM, processus Claude, credential ou service externe n’a été lancé. Aucun coût n’a été engagé. Les parcours précédemment prouvés n’ont pas été répétés.

## Inventaire statique limité aux entrypoints connus

- `run_mission_012_live_mcp_agent_e2e.py`, `build_agent_command` et `run` : le mode a encore une branche de construction provider dans le source, mais les gardes testées la bloquent avant recherche d’exécutable ou lecture d’état (CONT-199; vérification focalisée 2/2 dans ce lot).
- `run_mission_012_live_composed_e2e.py`, `build_stage_commands` et `run` : les deux gardes viennent d’être ajoutées et les refus sont testés dans le présent lot. L’option CLI garde `claude` comme sélection historique, mais `run()` la refuse avant tout travail; l’appelant doit choisir explicitement un chemin non-provider s’il veut exécuter les stages.
- `run_mission_012_live_agent_pr_e2e.py`, `run`/`validate_model_cost` : consomme un résumé et des reçus d’une exécution précédente; ses commandes lancent le vérificateur contrôlé, Forgejo et le gate de fusion, mais pas un fournisseur modèle. Sa validation de reçus Claude n’est donc pas une capacité de lancement.
- `run_mission_012_supervised_private_netns_e2e.py::_bwrap_command` fixe `--agent-adapter local-test-adapter`; ce wrapper ne choisit pas Claude. `run_forgejo_private_netns_webhook_e2e.py` masque le répertoire `.claude` dans son sandbox, ce qui n’est pas présenté comme une politique d’egress pour un fournisseur.

Cet inventaire est un examen source ciblé des scripts nommés, pas un balayage exhaustif de toutes commandes possibles sur la machine. Il ne prouve pas qu’un appelant externe ne puisse démarrer Claude directement, ni l’isolation du modèle. Aucun prompt, argument, environnement, terminal, log ou transcript d’une session Claude existante n’a été consulté.

## Portée ouverte

Le code ne met pas encore en œuvre une politique d’egress qui autorise le fournisseur sans donner à l’agent accès aux routes Forgejo, aux secrets ou aux fichiers privés. Le refus empêche seulement les entrypoints intégrés connus de feindre qu’une telle frontière existe. Le parcours LLM à PR/merge, le benchmark apparié, l’examen juridique/licences complet et l’autorité de fusion contre tous les writers restent non démontrés.
