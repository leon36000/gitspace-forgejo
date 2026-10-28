# CONTINUATION-312 — gate de mission et diagnostics des routes réseau

Date : 27 octobre 2026. Worktree produit fortement modifié, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Constat et changement

Après CONT-311, une recherche des consommateurs de `direct_forgejo_routes_blocked` a trouvé `run_mission_012_live_mcp_agent_e2e.py`. Ce runner compares les observations de sandbox au dictionnaire canonique. Il rejetait le nouveau champ `direct_forgejo_route_probe_errors` parce qu’il n’était ni validé ni exclu de cette comparaison. C’était un refus fail-closed de la forme actuelle, pas une preuve ambiguë acceptée, mais le nouveau contrat ne pouvait donc pas traverser ce gate.

Le runner appelle désormais `validate_direct_route_probe_errors` du wrapper de confiance par l’intermédiaire de `validate_sandbox_route_probe_observations`, puis exclut ce seul champ technique de la comparaison restante. Le validateur existant exige les routes `api`, `web` et `git_receive_pack`, seulement les quatre clés attendues, `error_category=os_error`, errno positif et `phase=connect`.

Une nouvelle régression de test échoue d’abord parce que l’entrée de validation du runner n’existe pas. Après implémentation, elle vérifie l’acceptation des trois erreurs de connexion, le refus d’un échec en phase `send`, le refus d’une route manquante et du champ absent. Le runner invoque exactement cette fonction avant la validation de forme complète.

## Vérifications

- `python3 tests/test_mission_012_live_mcp_agent_e2e.py AgentCommandTests.test_sandbox_route_gate_requires_all_routes_to_fail_at_connect` : une exécution initiale rouge avant implémentation, puis 1/1 vert après.
- `python3 tests/test_mission_012_live_mcp_agent_e2e.py` : **18/18** sur la source finale.
- `python3 -m py_compile scripts/run_mission_012_live_mcp_agent_e2e.py tests/test_mission_012_live_mcp_agent_e2e.py` : réussi.
- `git diff --check` : réussi.

La sortie rouge puis la sortie finale sont adjacentes (`red-test.log`, `green-test.log`). Versions et empreintes sources sont également jointes.

## Portée et limites

C’est une correction de compatibilité d’un gate, couverte par tests unitaires du runner; aucun processus sandboxé, serveur Forgejo, PR, fusion ou e2e n’a été lancé dans CONT-312. La preuve ne couvre pas une observation kernel du réseau; cette limite reste celle de CONT-311. Le runner secondaire `run_forgejo_agent_egress_live_e2e.py` a été inspecté : il enveloppe `socket.create_connection` dans le processus de test, exige un refus par route sur cette frontière et vérifie l’errno/port; il n’a pas été modifié. Aucun LLM/AgentOS, appel payant, service externe, code Rust/Forgejo, déploiement ou changement de production.
